// SPDX-License-Identifier: MPL-2.0

//! Seeded generators for the journal and interpolation property tests:
//! random [`TestElement`] trees (two kinds, depth ≤ 5, fan-out ≤ 6),
//! random mutator trees (every mutator variant, `RepeatWhile` and
//! `MapChildren`, channel-changing mutations and `SetToInherited`),
//! random anchors, and random sequences of journaled calls — walks and
//! updates — sharing one builder. Plus the tree snapshot every property
//! compares by.
//!
//! Every generator draws from a [`StdRng`] seeded with
//! `seed_from_u64`, so a failing seed reproduces exactly.

use crate::engine::anchor::Anchors;
use crate::engine::journal::JournalBuilder;
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::{Comparator, Predicate};
use crate::engine::tests::test_element::{TestElement, TestField, TestKind, TestMutation, TestMutator};
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use indextree::NodeId;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

/// How many seeds each property runs over.
pub const SEEDS: u64 = 500;

/// The highest channel the generators use, plus one. Small, so keys
/// collide and rows hold runs of equal keys.
const CHANNELS: usize = 3;

/// One node of a tree as a snapshot records it: its id, its parent and
/// its element.
pub type SnapshotRow = (NodeId, Option<NodeId>, TestElement);

/// Every node of `tree` in pre-order, with its parent and its element.
/// Two snapshots are equal exactly when the trees have the same nodes,
/// the same structure, the same sibling order and equal elements —
/// and `TestElement`'s state is all of its fields, so equal elements
/// are elements in the same state.
pub fn snapshot(tree: &Tree<TestElement>) -> Vec<SnapshotRow> {
    tree.descendants()
        .map(|id| {
            let node = &tree.arena()[id];
            (id, node.parent(), node.get().clone())
        })
        .collect()
}

/// The node ids of a snapshot, in order: its structure and sibling
/// order without the elements.
pub fn order(rows: &[SnapshotRow]) -> Vec<(NodeId, Option<NodeId>)> {
    rows.iter().map(|(id, parent, _)| (*id, *parent)).collect()
}

/// The inherited state `id`'s parent hands it: the sum of the values
/// from the root down to that parent (each `TestElement` hands its
/// children `parent + value`, and every walk here is seeded with 0 at
/// the root's parent). The `seed_for` every generated walk uses.
pub fn state_above(tree: &Tree<TestElement>, id: NodeId) -> i64 {
    id.ancestors(tree.arena())
        .skip(1)
        .map(|ancestor| tree.arena()[ancestor].get().value)
        .sum()
}

/// One journaled call of a [`Scenario`].
#[derive(Clone, Debug)]
pub enum Call {
    /// `mutators.apply_at_journaled(tree, anchors, state_above, builder)`.
    Walk {
        /// The mutator tree.
        mutators: MutatorTree<TestElement, TestMutation>,
        /// Where it applies.
        anchors: Anchors,
    },
    /// `tree.update_journaled(node, builder, |e| mutation.apply_to(e, &0))`.
    Update {
        /// The element changed.
        node: NodeId,
        /// What is done to it.
        mutation: TestMutation,
    },
}

impl Call {
    /// Run this call against `tree`, recording into `builder`.
    pub fn run(
        &self,
        tree: &mut Tree<TestElement>,
        builder: &mut JournalBuilder<TestElement>,
    ) -> Result<(), TreeError> {
        match self {
            Call::Walk { mutators, anchors } => {
                mutators.apply_at_journaled(tree, anchors, state_above, builder)
            }
            Call::Update { node, mutation } => {
                tree.update_journaled(*node, builder, |e| mutation.apply_to(e, &0))
            }
        }
    }
}

/// A random tree and a random sequence of journaled calls to make on
/// it with one builder.
#[derive(Clone, Debug)]
pub struct Scenario {
    /// The tree before any call.
    pub tree: Tree<TestElement>,
    /// The calls, in order.
    pub calls: Vec<Call>,
}

/// The scenario for `seed`: a random tree and one to five calls, each a
/// walk or an update.
///
/// The calls crowd one **busy row** — the children of a random node
/// that has at least two — and a small focus set of anchors: walks are
/// often anchored at the busy row's parent or at the focus set, and
/// updates mostly pick a busy-row child and half the time change its
/// channel. So one builder often applies to an element in several
/// calls, and often moves an element in one call next to an element
/// another call moves later — the case where a position read at the
/// later call's time would name a neighbor that has since moved.
pub fn scenario(seed: u64) -> Scenario {
    let mut rng = StdRng::seed_from_u64(seed);
    let tree = random_tree(&mut rng);
    let nodes: Vec<NodeId> = tree.descendants().collect();
    let parents: Vec<NodeId> = nodes
        .iter()
        .copied()
        .filter(|id| id.children(tree.arena()).nth(1).is_some())
        .collect();
    let busy = if parents.is_empty() {
        tree.root
    } else {
        parents[rng.random_range(0..parents.len())]
    };
    let row: Vec<NodeId> = busy.children(tree.arena()).collect();
    let focus = random_anchors(&mut rng, &nodes);
    let count = rng.random_range(1..=5);
    let calls = (0..count)
        .map(|_| {
            if rng.random_bool(0.55) {
                let anchors = match rng.random_range(0..5) {
                    0 | 1 => Anchors::Nodes(vec![busy]),
                    2 | 3 => focus.clone(),
                    _ => random_anchors(&mut rng, &nodes),
                };
                Call::Walk {
                    mutators: random_mutator_tree(&mut rng, &tree, &anchors, false),
                    anchors,
                }
            } else {
                let pool = match rng.random_range(0..5) {
                    0..=2 if !row.is_empty() => &row,
                    3 => focus_nodes(&focus),
                    _ => &nodes,
                };
                let mutation = if rng.random_bool(0.5) {
                    TestMutation::SetChannel(rng.random_range(0..CHANNELS))
                } else {
                    random_mutation(&mut rng)
                };
                Call::Update {
                    node: pool[rng.random_range(0..pool.len())],
                    mutation,
                }
            }
        })
        .collect();
    Scenario { tree, calls }
}

/// The nodes an anchor set is stated in terms of.
fn focus_nodes(anchors: &Anchors) -> &[NodeId] {
    match anchors {
        Anchors::Nodes(nodes) | Anchors::ParentOf(nodes) | Anchors::SiblingsOf(nodes) => nodes,
    }
}

/// A scenario for `seed` whose every call changes nothing: one walk
/// built from mutations that undo themselves within one apply (see
/// [`random_mutator_tree`]'s `no_op`).
pub fn no_op_scenario(seed: u64) -> Scenario {
    let mut rng = StdRng::seed_from_u64(seed);
    let tree = random_tree(&mut rng);
    let nodes: Vec<NodeId> = tree.descendants().collect();
    let anchors = random_anchors(&mut rng, &nodes);
    let calls = vec![Call::Walk {
        mutators: random_mutator_tree(&mut rng, &tree, &anchors, true),
        anchors,
    }];
    Scenario { tree, calls }
}

/// A random element: either kind, a channel below [`CHANNELS`], a
/// small value.
fn random_element(rng: &mut StdRng) -> TestElement {
    let channel = rng.random_range(0..CHANNELS);
    let value = rng.random_range(-5..=5);
    if rng.random_bool(0.5) {
        TestElement::new(channel, value)
    } else {
        TestElement::marked(channel, value)
    }
}

/// A random tree of depth ≤ 5 below its root and fan-out ≤ 6: up to six
/// children per node on the first two levels, up to three below, so a
/// tree stays in the hundreds of nodes.
pub fn random_tree(rng: &mut StdRng) -> Tree<TestElement> {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    grow(rng, &mut tree, root, 1);
    tree
}

fn grow(rng: &mut StdRng, tree: &mut Tree<TestElement>, parent: NodeId, depth: usize) {
    if depth > 5 {
        return;
    }
    let fan_out = if depth <= 2 {
        rng.random_range(0..=6)
    } else {
        rng.random_range(0..=3)
    };
    for _ in 0..fan_out {
        let element = random_element(rng);
        let at = if rng.random_bool(0.5) {
            Placement::Last
        } else {
            Placement::First
        };
        let child = tree.insert_child(parent, element, at).unwrap();
        grow(rng, tree, child, depth + 1);
    }
}

/// A random key: either kind, a channel below [`CHANNELS`].
fn random_key(rng: &mut StdRng) -> BranchKey<TestKind> {
    let kind = if rng.random_bool(0.5) {
        TestKind::Plain
    } else {
        TestKind::Marked
    };
    BranchKey::new(kind, rng.random_range(0..CHANNELS))
}

/// A random mutation, weighted so that a third change a key.
pub fn random_mutation(rng: &mut StdRng) -> TestMutation {
    match rng.random_range(0..9) {
        0..=2 => TestMutation::Add(rng.random_range(-3..=3)),
        3 => TestMutation::Set(rng.random_range(-5..=5)),
        4 | 5 => TestMutation::SetToInherited,
        _ => TestMutation::SetChannel(rng.random_range(0..CHANNELS)),
    }
}

/// A random payload for a mutator on `key`. With `no_op`, a payload
/// whose net effect on any target is nothing: `Add(0)`, an `Add` and its
/// inverse, or — on a key-merged node, where the target's channel is the
/// key's — a channel change and its inverse, all inside one apply.
fn random_payload(rng: &mut StdRng, key: BranchKey<TestKind>, no_op: bool, keyed: bool) -> TestMutator {
    if no_op {
        let n = rng.random_range(1..=4);
        return match rng.random_range(0..3) {
            0 => Mutator::new(TestMutation::Add(0), key),
            1 => Mutator::new_macro(vec![TestMutation::Add(n), TestMutation::Add(-n)], key),
            _ if keyed => Mutator::new_macro(
                vec![
                    TestMutation::SetChannel(CHANNELS + 2),
                    TestMutation::SetChannel(key.channel),
                ],
                key,
            ),
            _ => Mutator::new_macro(vec![TestMutation::Add(-n), TestMutation::Add(n)], key),
        };
    }
    if rng.random_bool(0.5) {
        Mutator::new(random_mutation(rng), key)
    } else {
        let len = rng.random_range(0..=3);
        Mutator::new_macro((0..len).map(|_| random_mutation(rng)).collect(), key)
    }
}

/// A random mutator node on `key`: a payload, a `Void`, or — not under
/// `no_op` — an instruction (`RepeatWhile` on a value bound, or
/// `MapChildren`) with or without a mutation of its own. `keyed` says
/// whether the walk reaches this node's targets by key, which only a
/// node with no instruction above it and no root position is.
fn random_mutator(rng: &mut StdRng, key: BranchKey<TestKind>, no_op: bool, keyed: bool) -> TestMutator {
    match rng.random_range(0..10) {
        0..=5 => random_payload(rng, key, no_op, keyed),
        6 => Mutator::new_void(key),
        _ if no_op => Mutator::new_void(key),
        7 | 8 => {
            let bound = rng.random_range(-4..=4);
            let comparator = if rng.random_bool(0.5) {
                Comparator::less()
            } else {
                Comparator::greater()
            };
            let mut predicate = Predicate::new();
            predicate.fields.push((TestField::Value(bound), comparator));
            Mutator::Instruction {
                key,
                instruction: Instruction::RepeatWhile(predicate),
                mutation: rng.random_bool(0.5).then(|| random_mutation(rng)),
            }
        }
        _ => Mutator::Instruction {
            key,
            instruction: Instruction::MapChildren,
            mutation: rng.random_bool(0.5).then(|| random_mutation(rng)),
        },
    }
}

/// A random mutator tree for `anchors` in `tree`: a root and up to
/// three levels of up to three children each. The root takes the kind
/// of one of the nodes the anchors name, most of the time, so that it
/// applies somewhere; every other key is random. With `no_op`, every
/// payload undoes itself and there are no instructions.
pub fn random_mutator_tree(
    rng: &mut StdRng,
    tree: &Tree<TestElement>,
    anchors: &Anchors,
    no_op: bool,
) -> MutatorTree<TestElement, TestMutation> {
    let mut key = random_key(rng);
    let named = focus_nodes(anchors);
    if rng.random_bool(0.8) {
        let node = named[rng.random_range(0..named.len())];
        let target = match anchors {
            Anchors::ParentOf(_) => tree.arena()[node].parent().unwrap_or(node),
            _ => node,
        };
        key.kind = tree.arena()[target].get().kind;
    }
    let root = random_mutator(rng, key, no_op, false);
    let keyed_below = !matches!(root, Mutator::Instruction { .. });
    let mut mutators = MutatorTree::new_with(root);
    let root = mutators.root;
    sprout(rng, &mut mutators, root, 1, no_op, keyed_below);
    mutators
}

fn sprout(
    rng: &mut StdRng,
    mutators: &mut MutatorTree<TestElement, TestMutation>,
    parent: NodeId,
    depth: usize,
    no_op: bool,
    keyed: bool,
) {
    if depth > 3 {
        return;
    }
    for _ in 0..rng.random_range(1..=3) {
        let key = random_key(rng);
        let mutator = random_mutator(rng, key, no_op, keyed);
        let keyed_below = keyed && !matches!(mutator, Mutator::Instruction { .. });
        let child = mutators.insert_child(parent, mutator, Placement::Last).unwrap();
        sprout(rng, mutators, child, depth + 1, no_op, keyed_below);
    }
}

/// Random anchors over `nodes`: one to three of them as `Nodes` most of
/// the time, otherwise as `ParentOf` or `SiblingsOf`.
pub fn random_anchors(rng: &mut StdRng, nodes: &[NodeId]) -> Anchors {
    let count = rng.random_range(1..=3);
    let picked = (0..count)
        .map(|_| nodes[rng.random_range(0..nodes.len())])
        .collect();
    match rng.random_range(0..6) {
        0 => Anchors::ParentOf(picked),
        1 => Anchors::SiblingsOf(picked),
        _ => Anchors::Nodes(picked),
    }
}
