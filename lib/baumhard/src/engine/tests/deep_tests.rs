// SPDX-License-Identifier: MPL-2.0

//! Depth costs nothing extra (design D30, §20 rule 11; V1-A5, #282):
//! every walk the engine makes survives a single chain [`DEEP`]
//! elements deep on a default test thread — and, through the wasm32
//! node probe (`tests/wasm_font_lock.rs`, TEST_CONVENTIONS §T9), on
//! wasm32's default stack in a debug build — and spends no work per
//! element that grows with the depth.
//!
//! The `do_*` bodies are the survival half: a full walk, a
//! `RepeatWhile` descent to the leaf, anchored walks at the leaf and at
//! every element, a journaled edit reverted and replayed, a structural
//! move of the whole chain, a deep subtree inserted, and a deep
//! [`Subtree`] dropped, cloned and compared. Each would overflow the
//! stack if its walk recursed once per level. The plain tests below
//! them are the cost half: each counts the steps of one mechanism —
//! inherited state, anchor climbs, a journal's close, a same-row
//! re-attach — on a chain of [`STEPS_DEEP`], so a per-element climb
//! to the root shows as a count proportional to the square of the
//! depth, never as a time.

use crate::engine::anchor::Anchors;
use crate::engine::journal::JournalBuilder;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::{Comparator, Predicate};
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tests::test_element::{
    add, add_mutator, inherit_calls, plain, value, TestElement, TestField, TestMutation,
};
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use indextree::NodeId;

/// How deep the survival chains go: §20 rule 11's chain.
pub const DEEP: usize = 100_000;

/// How deep the step-count chains go: deep enough that a climb per
/// element would be hundreds of millions of steps, shallow enough that
/// a control which replants one still finishes.
pub const STEPS_DEEP: usize = 20_000;

/// A tree whose root (the void element, value 0) holds a chain of
/// `depth` elements, each a `Plain` element on channel 0 with value 1,
/// except the leaf, whose value is `leaf`. Returns the tree and the
/// chain from the top down; built by appending, so building it climbs
/// nothing.
pub fn chain(depth: usize, leaf: i64) -> (Tree<TestElement>, Vec<NodeId>) {
    let mut tree = Tree::new_non_indexed();
    let mut at = tree.root;
    let mut ids = Vec::with_capacity(depth);
    for level in 0..depth {
        let v = if level + 1 == depth { leaf } else { 1 };
        at = add(&mut tree, at, TestElement::new(0, v));
        ids.push(at);
    }
    (tree, ids)
}

/// A mutator tree that mirrors a chain `depth` elements deep under its
/// `Void` root: `Add(1)` at every level, and `last` at the bottom.
fn mirrored(depth: usize, last: TestMutation) -> MutatorTree<TestElement, TestMutation> {
    let mut mutators = MutatorTree::new();
    let mut at = mutators.root;
    for level in 0..depth {
        let mutation = if level + 1 == depth {
            last.clone()
        } else {
            TestMutation::Add(1)
        };
        at = add_mutator(&mut mutators, at, Mutator::new(mutation, plain(0)));
    }
    mutators
}

/// A mutator tree of one node on `plain(0)`.
fn single(mutation: TestMutation) -> MutatorTree<TestElement, TestMutation> {
    MutatorTree::new_with(Mutator::new(mutation, plain(0)))
}

/// The state `id`'s parent hands it: the sum of the values above it.
/// The oracle, by its own climb — asked once per test, never per
/// element.
fn sum_above(tree: &Tree<TestElement>, id: NodeId) -> i64 {
    id.ancestors(tree.arena()).skip(1).map(|a| value(tree, a)).sum()
}

/// A [`Subtree`] chain `depth` elements deep, values `1..=depth` from
/// the top, built from the bottom up so building it recurses on
/// nothing.
pub fn deep_subtree(depth: usize) -> Subtree<TestElement> {
    let mut subtree = Subtree::leaf(TestElement::new(0, depth as i64));
    for v in (1..depth as i64).rev() {
        subtree = Subtree {
            root: TestElement::new(0, v),
            children: vec![subtree],
        };
    }
    subtree
}

/// The bottom of a subtree chain, mutably.
fn bottom_of(subtree: &mut Subtree<TestElement>) -> &mut Subtree<TestElement> {
    let mut at = subtree;
    while !at.children.is_empty() {
        at = &mut at.children[0];
    }
    at
}

#[test]
fn test_engine_deep_chain_full_walk() {
    do_engine_deep_chain_full_walk();
}

/// A mutator as deep as the chain walks all of it from the root:
/// `Add(1)` reaches every element, and the leaf's `SetToInherited`
/// reads the state carried down 100,000 levels — the sum of every
/// value above it, after their own `Add`s.
pub fn do_engine_deep_chain_full_walk() {
    let (mut tree, ids) = chain(DEEP, 1);
    let mutators = mirrored(DEEP, TestMutation::SetToInherited);
    mutators.apply_to(&mut tree, &0);
    assert_eq!(value(&tree, ids[0]), 2, "the top took its Add");
    assert_eq!(
        value(&tree, ids[DEEP - 2]),
        2,
        "the level above the leaf took its Add"
    );
    assert_eq!(
        value(&tree, ids[DEEP - 1]),
        2 * (DEEP as i64 - 1),
        "the leaf holds the state carried down every level"
    );
}

#[test]
fn test_engine_deep_chain_repeat_while_to_the_leaf() {
    do_engine_deep_chain_repeat_while_to_the_leaf();
}

/// A `RepeatWhile` that matches every element but the leaf descends the
/// whole chain, re-keying every element on the way — a settle at every
/// level — and its terminator runs at the leaf, 100,000 levels down.
pub fn do_engine_deep_chain_repeat_while_to_the_leaf() {
    let (mut tree, ids) = chain(DEEP, 5);
    let predicate = Predicate {
        fields: vec![(TestField::Value(1), Comparator::equals())],
        always_match: false,
    };
    let mut mutators = MutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(predicate),
        plain(0),
    ));
    let repeat = mutators.root;
    let repeated = add_mutator(
        &mut mutators,
        repeat,
        Mutator::new_macro(vec![TestMutation::Add(10), TestMutation::SetChannel(3)], plain(0)),
    );
    add_mutator(
        &mut mutators,
        repeated,
        Mutator::new(TestMutation::Set(42), plain(0)),
    );
    mutators.apply_to(&mut tree, &0);
    assert_eq!(value(&tree, ids[0]), 11, "the top matched");
    assert_eq!(
        value(&tree, ids[DEEP - 2]),
        11,
        "the level above the leaf matched"
    );
    assert_eq!(
        tree.arena()[ids[DEEP - 2]].get().channel,
        3,
        "re-keyed, and settled in place"
    );
    assert_eq!(value(&tree, ids[DEEP - 1]), 42, "the terminator ran at the leaf");
    assert_eq!(tree.check_sorted(), Ok(()));
}

#[test]
fn test_engine_deep_chain_anchored_walks() {
    do_engine_deep_chain_anchored_walks();
}

/// An anchored walk at the leaf, seeded with the state above it; a
/// walk anchored at every element of the chain, which walks each one;
/// and a reaching walk anchored at every element, which drops every
/// anchor nested in another and walks the top alone.
pub fn do_engine_deep_chain_anchored_walks() {
    let (mut tree, ids) = chain(DEEP, 1);
    let leaf = ids[DEEP - 1];
    single(TestMutation::SetToInherited)
        .apply_at(&mut tree, &Anchors::Nodes(vec![leaf]), sum_above)
        .unwrap();
    assert_eq!(
        value(&tree, leaf),
        DEEP as i64 - 1,
        "the seed is the state above the leaf"
    );

    let (mut tree, ids) = chain(DEEP, 1);
    single(TestMutation::Add(1))
        .apply_at(
            &mut tree,
            &Anchors::Nodes(ids.iter().rev().copied().collect()),
            |_, _| 0,
        )
        .unwrap();
    assert!(
        ids.iter().all(|&id| value(&tree, id) == 2),
        "a mutator that reaches only its root walks every anchor"
    );

    let mut reaching = single(TestMutation::Add(1));
    let below = reaching.root;
    add_mutator(
        &mut reaching,
        below,
        Mutator::new(TestMutation::Add(100), plain(0)),
    );
    reaching
        .apply_at(&mut tree, &Anchors::Nodes(ids.clone()), |_, _| 0)
        .unwrap();
    assert_eq!(value(&tree, ids[0]), 3, "the top anchor is walked");
    assert_eq!(value(&tree, ids[1]), 102, "its child, reached through it, once");
    assert_eq!(value(&tree, ids[2]), 2, "every nested anchor is dropped");
}

#[test]
fn test_engine_deep_chain_journaled_edit_at_the_leaf() {
    do_engine_deep_chain_journaled_edit_at_the_leaf();
}

/// A journaled edit at the leaf — a value and a key — and a journaled
/// edit of every element plus an insert below the leaf, in one builder:
/// closed, reverted and replayed exactly, 100,000 levels down.
pub fn do_engine_deep_chain_journaled_edit_at_the_leaf() {
    let (mut tree, ids) = chain(DEEP, 1);
    let leaf = ids[DEEP - 1];
    let mut builder = JournalBuilder::new();
    MutatorTree::new_with(Mutator::new_macro(
        vec![TestMutation::Add(5), TestMutation::SetChannel(2)],
        plain(0),
    ))
    .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![leaf]), |_, _| 0, &mut builder)
    .unwrap();
    let root = tree.root;
    repeating(TestMutation::Add(1))
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    single_insert(TestElement::new(0, 7))
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![leaf]), |_, _| 0, &mut builder)
        .unwrap();
    let journal = builder.close(&tree);
    let inserted = tree.arena()[leaf]
        .first_child()
        .expect("the insert is below the leaf");
    assert_eq!(value(&tree, leaf), 1 + 5 + 1);
    assert_eq!(value(&tree, ids[0]), 2);

    journal.revert(&mut tree).unwrap();
    assert_eq!(value(&tree, leaf), 1, "reverted");
    assert_eq!(tree.arena()[leaf].get().channel, 0, "its key reverted");
    assert_eq!(value(&tree, ids[0]), 1, "the top reverted");
    assert!(tree.arena()[leaf].first_child().is_none(), "the insert is out");

    journal.replay(&mut tree).unwrap();
    assert_eq!(value(&tree, leaf), 7, "replayed");
    assert_eq!(tree.arena()[leaf].get().channel, 2);
    assert_eq!(
        tree.arena()[leaf].first_child(),
        Some(inserted),
        "the same insert is back"
    );
    journal.discard(&mut tree);
    assert_eq!(tree.check_sorted(), Ok(()));
}

/// A mutator tree of one insert of `element` as a leaf, on `plain(0)`.
fn single_insert(element: TestElement) -> MutatorTree<TestElement, TestMutation> {
    MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree::leaf(element),
            at: Placement::Last,
        },
        plain(0),
    ))
}

#[test]
fn test_engine_deep_chain_moves_whole() {
    do_engine_deep_chain_moves_whole();
}

/// The whole chain moves under a new parent, journaled, and is reverted
/// and replayed; moving it under its own leaf is a cycle, refused with
/// nothing changed.
pub fn do_engine_deep_chain_moves_whole() {
    let (mut tree, ids) = chain(DEEP, 1);
    let root = tree.root;
    let top = ids[0];
    let leaf = ids[DEEP - 1];
    let other = add(&mut tree, root, TestElement::new(0, 9));
    let moving = |to| -> MutatorTree<TestElement, TestMutation> {
        MutatorTree::new_with(Mutator::new_structure(
            StructOp::Move {
                to,
                at: Placement::Last,
            },
            plain(0),
        ))
    };

    let mut builder = JournalBuilder::new();
    moving(other)
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![top]), |_, _| 0, &mut builder)
        .unwrap();
    let journal = builder.close(&tree);
    assert_eq!(tree.arena()[top].parent(), Some(other), "moved");
    assert_eq!(leaf.ancestors(tree.arena()).count(), DEEP + 2);
    journal.revert(&mut tree).unwrap();
    assert_eq!(tree.arena()[top].parent(), Some(root), "reverted");
    journal.replay(&mut tree).unwrap();
    assert_eq!(tree.arena()[top].parent(), Some(other), "replayed");
    journal.discard(&mut tree);

    let refused = moving(leaf).apply_at(&mut tree, &Anchors::Nodes(vec![top]), |_, _| 0);
    assert_eq!(refused, Err(TreeError::Cycle { node: top, to: leaf }));
    assert_eq!(tree.arena()[top].parent(), Some(other), "nothing changed");
}

#[test]
fn test_engine_deep_subtree_inserts_and_is_a_value() {
    do_engine_deep_subtree_inserts_and_is_a_value();
}

/// A [`Subtree`] 100,000 deep is inserted below the leaf of a chain as
/// deep — 200,000 levels — journaled, reverted, replayed and discarded;
/// and the subtree itself is cloned, compared and dropped on this
/// thread.
pub fn do_engine_deep_subtree_inserts_and_is_a_value() {
    let subtree = deep_subtree(DEEP);
    let copy = subtree.clone();
    assert!(copy == subtree, "a clone is equal");
    let mut changed = subtree.clone();
    bottom_of(&mut changed).root.value += 1;
    assert!(changed != subtree, "a change at the bottom is a difference");
    assert_eq!(
        bottom_of(&mut changed).root.value,
        DEEP as i64 + 1,
        "precondition: the change is at the bottom"
    );
    drop(changed);
    drop(copy);

    let (mut tree, ids) = chain(DEEP, 1);
    let leaf = ids[DEEP - 1];
    let insert: MutatorTree<TestElement, TestMutation> = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree,
            at: Placement::Last,
        },
        plain(0),
    ));
    let mut builder = JournalBuilder::new();
    insert
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![leaf]), |_, _| 0, &mut builder)
        .unwrap();
    let journal = builder.close(&tree);
    let top = tree.arena()[leaf].first_child().expect("inserted below the leaf");
    let mut bottom = top;
    while let Some(next) = tree.arena()[bottom].first_child() {
        bottom = next;
    }
    assert_eq!(value(&tree, bottom), DEEP as i64, "the whole subtree is in");
    assert_eq!(
        journal.touched().count(),
        DEEP,
        "every inserted element is touched"
    );
    journal.revert(&mut tree).unwrap();
    assert!(tree.arena()[leaf].first_child().is_none(), "reverted");
    journal.replay(&mut tree).unwrap();
    assert_eq!(tree.arena()[leaf].first_child(), Some(top), "replayed, same ids");
    journal.discard(&mut tree);
    drop(insert);
}

/// A mutator tree whose root repeats `mutation` over every element
/// below the element it lands on.
fn repeating(mutation: TestMutation) -> MutatorTree<TestElement, TestMutation> {
    let mut mutators = MutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(Predicate::always_true()),
        plain(0),
    ));
    let root = mutators.root;
    add_mutator(&mut mutators, root, Mutator::new(mutation, plain(0)));
    mutators
}

#[test]
fn test_engine_deep_walks_inherit_once_per_level() {
    do_engine_deep_walks_inherit_once_per_level();
}

/// The walks carry inherited state down: a full walk, a `RepeatWhile`
/// descent and a `MapChildren` zip each compute it once per element
/// they descend through, counted in calls to `inherit`, not timed. A
/// walk that re-derived each element's state by folding down from the
/// root would make `STEPS_DEEP²/2` calls.
///
/// The counts are exact. The full walk and the descent each compute the
/// state the root hands down and the state of every element but the
/// leaf, which has no children to hand one to: `STEPS_DEEP`. The zip
/// computes the state below every element it pairs, the leaf's
/// included — a `MapChildren` computes it before it looks for children —
/// so one more.
pub fn do_engine_deep_walks_inherit_once_per_level() {
    let (mut tree, _) = chain(STEPS_DEEP, 1);
    let before = inherit_calls();
    mirrored(STEPS_DEEP, TestMutation::SetToInherited).apply_to(&mut tree, &0);
    let full = inherit_calls() - before;
    assert_eq!(full, STEPS_DEEP, "inherits for a full walk {STEPS_DEEP} deep");

    let before = inherit_calls();
    repeating(TestMutation::SetToInherited).apply_to(&mut tree, &0);
    let repeat = inherit_calls() - before;
    assert_eq!(repeat, STEPS_DEEP, "inherits for a descent {STEPS_DEEP} deep");

    let mut zipped = MutatorTree::new();
    let mut at = zipped.root;
    for _ in 0..STEPS_DEEP {
        at = add_mutator(
            &mut zipped,
            at,
            Mutator::Instruction {
                key: plain(0),
                instruction: Instruction::MapChildren,
                mutation: Some(TestMutation::SetToInherited),
            },
        );
    }
    let before = inherit_calls();
    zipped.apply_to(&mut tree, &0);
    let zip = inherit_calls() - before;
    assert_eq!(zip, STEPS_DEEP + 1, "inherits for a zip {STEPS_DEEP} deep");
}

// The step counts below read counters the engine keeps only in test
// builds (`tests_support` in `membership`, `journal` and `siblings`),
// so these bodies cannot compile into the library and stay plain
// tests (lib/baumhard/CONVENTIONS.md §B8, the first shape).

/// An anchored walk at every element of a chain climbs through each
/// element once — to check that every anchor is in the tree and to put
/// the anchors in tree order — not once per anchor: exactly
/// `STEPS_DEEP` climb steps, one per element below the root (the root
/// answers without a step). A climb per anchor would be `STEPS_DEEP²/2`
/// of them, and a second memo for the ordering `2 × STEPS_DEEP`.
#[test]
fn test_engine_deep_anchors_climb_each_element_once() {
    use crate::engine::membership::tests_support::climbed_steps;
    let (mut tree, ids) = chain(STEPS_DEEP, 1);
    let before = climbed_steps();
    single(TestMutation::Add(1))
        .apply_at(
            &mut tree,
            &Anchors::Nodes(ids.iter().rev().copied().collect()),
            |_, _| 0,
        )
        .unwrap();
    let climbed = climbed_steps() - before;
    assert!(
        ids.iter().all(|&id| value(&tree, id) == 2),
        "precondition: every anchor was walked"
    );
    assert_eq!(
        climbed, STEPS_DEEP,
        "climb steps for {STEPS_DEEP} anchors on one chain"
    );
}

/// A journal that edits every element of a chain and inserts below its
/// leaf has every entry above a node that entered the tree; its close
/// asks, for every entry, the top of what entered with it — and climbs
/// through each element once over all of them, counted in steps.
///
/// Exactly `2 × STEPS_DEEP + 3`: the climb up the arrangement after goes
/// through every chain element, the root and the inserted node
/// (`STEPS_DEEP + 2`), and the climb up the arrangement before through
/// every chain element and the root (`STEPS_DEEP + 1`) — the inserted
/// node was not in the tree before, so nothing asks where it left from.
#[test]
fn test_engine_deep_journal_close_climbs_each_element_once() {
    use crate::engine::journal::tests_support::presence_steps;
    let (mut tree, ids) = chain(STEPS_DEEP, 1);
    let leaf = ids[STEPS_DEEP - 1];
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    repeating(TestMutation::Add(1))
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    single_insert(TestElement::new(0, 7))
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![leaf]), |_, _| 0, &mut builder)
        .unwrap();
    let before = presence_steps();
    let journal = builder.close(&tree);
    let steps = presence_steps() - before;
    assert_eq!(
        journal.touched().count(),
        STEPS_DEEP + 1,
        "precondition: every element is an entry, and the insert is one more"
    );
    assert_eq!(
        steps,
        2 * STEPS_DEEP + 3,
        "climb steps to close {STEPS_DEEP} entries on one chain"
    );
    journal.discard(&mut tree);
}

/// A chain two wide — every element holding the next one and a leaf —
/// whose every row a walk re-arranges, then reverts and replays: each
/// re-arranged element goes back into the row it left, which cannot
/// close a cycle, so the insert that puts it there must not climb to
/// the root (ruling R16 on #282). Counted in the steps of every
/// same-row insert.
///
/// Ignored until V1-A6 (#290) replaces indextree with the engine's own
/// arena tree, whose same-row insert makes no cycle check: indextree 4.9
/// has no insert of an existing node without its O(depth) one. It fails
/// today, by design; it is not counted as passing.
#[test]
#[ignore = "#290 (V1-A6): same-row re-attach is O(depth) until the engine's own arena tree replaces indextree"]
fn test_same_row_rearrangement_is_linear_in_depth() {
    use crate::engine::siblings::tests_support::row_link_steps;
    const WIDE_DEEP: usize = 5_000;
    let mut tree = Tree::new_non_indexed();
    let mut at = tree.root;
    for _ in 0..WIDE_DEEP {
        let next = add(&mut tree, at, TestElement::new(0, 1));
        add(&mut tree, at, TestElement::new(1, 1));
        at = next;
    }
    let root = tree.root;
    let before = row_link_steps();
    let mut builder = JournalBuilder::new();
    // Every element goes to channel 2: in each row the chain element,
    // on 0, and the leaf, on 1, are both re-keyed, so the settle
    // re-arranges both.
    repeating(TestMutation::SetChannel(2))
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    journal.replay(&mut tree).unwrap();
    let steps = row_link_steps() - before;
    assert_eq!(tree.check_sorted(), Ok(()), "precondition: the rows are sorted");
    assert!(steps > 0, "precondition: rows were re-arranged");
    assert!(
        steps <= 16 * WIDE_DEEP,
        "{steps} insert steps to re-arrange {WIDE_DEEP} rows on one chain"
    );
}

/// A walk that re-keys every element of a chain — each alone in its row
/// — settles without re-attaching any of them: an only child is in its
/// sorted place whatever its key. Counted in same-row insert steps,
/// which a settle that detached and re-attached each one would spend
/// as the depth per element, `STEPS_DEEP²/2` in all.
#[test]
fn test_engine_deep_settle_leaves_only_children_in_place() {
    use crate::engine::siblings::tests_support::row_link_steps;
    let (mut tree, ids) = chain(STEPS_DEEP, 1);
    let before = row_link_steps();
    repeating(TestMutation::SetChannel(4)).apply_to(&mut tree, &0);
    let steps = row_link_steps() - before;
    assert!(
        ids.iter().all(|&id| tree.arena()[id].get().channel == 4),
        "precondition: every element was re-keyed"
    );
    assert_eq!(
        steps, 0,
        "{steps} insert steps to settle {STEPS_DEEP} only children"
    );
}

/// The engine and the document model climb, and insert an existing
/// node, only where they may: indextree's climb in `tree::climb`, its
/// checked insert behind `siblings::checked_attach` — both counted in
/// test builds — and a hand-written climb (a parent read in the
/// spellings `engine_depth_cost_calls` reads) only in the memoized
/// `Membership::contains`, `Presence::top` and `resolve`, the walker's
/// one-off `state_above`, and the document's `tops_climbing`,
/// `state_of` and `find_binding`. Any of them written anywhere else in
/// the shipped code of `engine/` or `document/` is a cost the counts
/// below could not see, so it is refused by name.
#[test]
fn test_the_engine_climbs_only_where_it_may() {
    let costs = crate::util::source_scan::engine_depth_cost_calls();
    let unused: Vec<_> = costs.allowed.iter().filter(|(_, &calls)| calls == 0).collect();
    assert!(
        unused.is_empty() && costs.allowed.len() == 10,
        "precondition: each of the ten allowed functions makes a call the scan reads; \
         unused: {unused:?}"
    );
    let found = costs.found;
    assert!(
        found.is_empty(),
        "a climb, or an insert of an existing node, outside the functions allowed to \
         make one, which no step count sees:\n{}",
        found
            .iter()
            .map(|(file, line, code)| format!("  {file}:{line}   {code}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Structural commands and updates on a deep chain climb nothing and
/// insert no existing node — counted, not timed:
/// - a [`Subtree`] `STEPS_DEEP` deep inserted below the leaf (its every
///   node new, so put in by value);
/// - a `RepeatWhile` whose repeated mutator is a structural `Insert`,
///   which collects a command at every element of the chain and of the
///   inserted subtree, each checked against what earlier commands
///   removed in O(1);
/// - and a thousand updates of the leaf, plain and journaled, each
///   checking in O(1) that the leaf is in the tree.
///
/// Exactly 0 climb steps and 0 checked inserts. A climb per command, a
/// checked insert per new node or a `Tree::contains` that climbed would
/// each cost the depth per element here.
#[test]
fn test_engine_deep_commands_and_updates_climb_nothing() {
    use crate::engine::siblings::tests_support::checked_attaches;
    use crate::engine::tree::tests_support::climb_steps;
    let (mut tree, ids) = chain(STEPS_DEEP, 1);
    let leaf = ids[STEPS_DEEP - 1];
    let root = tree.root;
    let (climbs, attaches) = (climb_steps(), checked_attaches());

    MutatorTree::<TestElement, TestMutation>::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: deep_subtree(STEPS_DEEP),
            at: Placement::Last,
        },
        plain(0),
    ))
    .apply_at(&mut tree, &Anchors::Nodes(vec![leaf]), |_, _| 0)
    .unwrap();

    let mut every = MutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(Predicate::always_true()),
        plain(0),
    ));
    let repeat = every.root;
    add_mutator(
        &mut every,
        repeat,
        Mutator::new_structure(
            StructOp::Insert {
                subtree: Subtree::leaf(TestElement::new(5, 0)),
                at: Placement::Last,
            },
            plain(0),
        ),
    );
    let mut builder = JournalBuilder::new();
    every
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    for _ in 0..1_000 {
        tree.update(leaf, |element| element.value += 1).unwrap();
        tree.update_journaled(leaf, &mut builder, |element| element.value += 1)
            .unwrap();
    }
    let journal = builder.close(&tree);

    assert_eq!(
        tree.descendants().count(),
        1 + 2 * STEPS_DEEP + 2 * STEPS_DEEP,
        "precondition: the subtree and a leaf below every element are in"
    );
    assert_eq!(value(&tree, leaf), 1 + 2_000, "precondition: every update ran");
    assert_eq!(climb_steps() - climbs, 0, "climb steps");
    assert_eq!(
        checked_attaches() - attaches,
        0,
        "checked inserts of existing nodes"
    );
    journal.discard(&mut tree);
}

/// `Tree::contains` stays exact through a removal, its revert, its
/// replay and the discard that frees it, for every element of the
/// removed subtree — the set of detached nodes behind it is written
/// from the journal's touches, never by a climb.
pub fn do_engine_contains_follows_removal_revert_replay_and_discard() {
    let (mut tree, ids) = chain(STEPS_DEEP, 1);
    let middle = ids[STEPS_DEEP / 2];
    let leaf = ids[STEPS_DEEP - 1];
    let remove = MutatorTree::<TestElement, TestMutation>::new_with(Mutator::new_structure(
        StructOp::Remove,
        plain(0),
    ));
    let mut builder = JournalBuilder::new();
    remove
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![middle]), |_, _| 0, &mut builder)
        .unwrap();
    let journal = builder.close(&tree);
    let out = |tree: &Tree<TestElement>| !tree.contains(middle) && !tree.contains(leaf);
    let inside = |tree: &Tree<TestElement>| tree.contains(middle) && tree.contains(leaf);
    assert!(out(&tree), "removed");
    assert!(tree.contains(ids[STEPS_DEEP / 2 - 1]), "the parent stays");
    assert_eq!(
        tree.update(leaf, |e| e.value = 9),
        Err(TreeError::StaleNode(leaf)),
        "an update of a removed node is refused"
    );
    journal.revert(&mut tree).unwrap();
    assert!(inside(&tree), "reverted");
    journal.replay(&mut tree).unwrap();
    assert!(out(&tree), "replayed");
    journal.revert(&mut tree).unwrap();
    assert!(inside(&tree), "reverted again");
    journal.replay(&mut tree).unwrap();
    journal.discard(&mut tree);
    assert!(out(&tree), "discarded");
    assert!(tree.arena().get(leaf).is_none(), "and freed");
    assert!(tree.detached.is_empty(), "nothing freed is still marked");
}

#[test]
fn test_engine_contains_follows_removal_revert_replay_and_discard() {
    do_engine_contains_follows_removal_revert_replay_and_discard();
}

/// What `#[derive(Clone, Debug, PartialEq)]` makes of a subtree: the
/// shape `Subtree` had before its impls were written by hand, held
/// beside them as the reference. Named apart, so its derived `Debug`
/// writes `Mirror` where `Subtree`'s writes `Subtree`.
#[derive(Clone, Debug, PartialEq)]
struct Mirror {
    root: TestElement,
    children: Vec<Mirror>,
}

/// A random subtree up to `depth` levels and three children a node,
/// with its derived mirror, from `rng`.
fn random_pair(rng: &mut rand::rngs::StdRng, depth: usize) -> (Subtree<TestElement>, Mirror) {
    use rand::RngExt;
    let root = if rng.random_range(0..2) == 0 {
        TestElement::new(rng.random_range(0..3), rng.random_range(-2..3))
    } else {
        TestElement::marked(rng.random_range(0..3), rng.random_range(-2..3))
    };
    let count = if depth == 0 { 0 } else { rng.random_range(0..=3) };
    let (children, mirrors) = (0..count).map(|_| random_pair(rng, depth - 1)).unzip();
    (
        Subtree {
            root: root.clone(),
            children,
        },
        Mirror {
            root,
            children: mirrors,
        },
    )
}

#[test]
fn test_engine_subtree_impls_match_the_derived_ones() {
    do_engine_subtree_impls_match_the_derived_ones();
}

/// `Subtree`'s hand-written `Debug` (`{:?}` and `{:#?}`), `Clone` and
/// `PartialEq` agree with what the derives make, on 2,000 random
/// subtrees up to five levels deep: the text is byte for byte the
/// derived text with the type's name, a clone is equal and prints the
/// same, and two subtrees are equal exactly when their mirrors are.
pub fn do_engine_subtree_impls_match_the_derived_ones() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(282);
    let mut previous: Option<(Subtree<TestElement>, Mirror)> = None;
    let mut equal_pairs = 0;
    for _ in 0..2_000 {
        let (subtree, mirror) = random_pair(&mut rng, 5);
        let derived = |text: String| text.replace("Mirror {", "Subtree {");
        assert_eq!(format!("{subtree:?}"), derived(format!("{mirror:?}")));
        assert_eq!(format!("{subtree:#?}"), derived(format!("{mirror:#?}")));
        let copy = subtree.clone();
        assert!(copy == subtree, "a clone is equal");
        assert_eq!(format!("{copy:#?}"), format!("{subtree:#?}"));
        if let Some((other, other_mirror)) = &previous {
            assert_eq!(
                subtree == *other,
                mirror == *other_mirror,
                "{subtree:?} against {other:?}"
            );
            equal_pairs += usize::from(mirror == *other_mirror);
        }
        previous = Some((subtree, mirror));
    }
    assert!(
        equal_pairs > 0,
        "precondition: some pairs are equal, so `==` is tested both ways"
    );
}
