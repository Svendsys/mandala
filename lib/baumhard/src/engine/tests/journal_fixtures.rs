// SPDX-License-Identifier: MPL-2.0

//! Seeded generators for the journal and interpolation property tests:
//! random [`TestElement`] trees (two kinds, depth ≤ 5, fan-out ≤ 6),
//! random mutator trees (every mutator variant, `RepeatWhile` and
//! `MapChildren`, channel-changing mutations and `SetToInherited`),
//! random anchors, and random sequences of journaled calls — walks and
//! updates — sharing one builder. Plus the tree snapshot every property
//! compares by.
//!
//! [`StructuralScenario`] extends the sequences with the structural
//! commands — insert, remove, move, split and merge, valid and invalid —
//! mixed with key changes and value changes across the calls of one
//! builder. Its calls are generated one at a time against the tree as
//! the earlier calls left it, so a command can name a node an earlier
//! call inserted or moved; [`run_structural`] runs one and checks what
//! every call must leave behind.
//!
//! Every generator draws from a [`StdRng`] seeded with
//! `seed_from_u64`, so a failing seed reproduces exactly.

use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::journal::{Journal, JournalBuilder};
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::{Comparator, Predicate};
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tests::test_element::{TestElement, TestField, TestKind, TestMutation, TestMutator};
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use indextree::NodeId;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use rustc_hash::{FxHashMap, FxHashSet};

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

/// The scenario for `seed`: a random tree and one to six calls, each a
/// walk or an update.
///
/// The calls crowd one **busy row** — the children of a random node
/// that has at least two — and a small focus set of anchors: walks are
/// often anchored at the busy row's parent or at the focus set, and
/// updates mostly pick a busy-row child, sometimes a node of the focus
/// set, and usually change its channel. So one builder often applies to an element in several
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
    let count = rng.random_range(1..=6);
    let calls = (0..count)
        .map(|_| {
            if rng.random_bool(0.5) {
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
                let pool = match rng.random_range(0..6) {
                    0..=3 if !row.is_empty() => &row,
                    4 => focus_nodes(&focus),
                    _ => &nodes,
                };
                let mutation = if rng.random_bool(0.7) {
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

// ── structural scenarios ──────────────────────────────────────────

/// What a generated call of a [`StructuralScenario`] sets out to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    /// A walk of field and key mutations, or an update.
    Field,
    /// A walk whose root inserts a subtree.
    Insert,
    /// A walk whose root removes its target.
    Remove,
    /// A walk whose root moves its target.
    Move,
    /// A walk whose root splits its target.
    Split,
    /// A walk whose root merges its target with the next sibling.
    Merge,
}

impl Intent {
    /// The structural intents, one per command.
    pub const STRUCTURAL: [Intent; 5] = [
        Intent::Insert,
        Intent::Remove,
        Intent::Move,
        Intent::Split,
        Intent::Merge,
    ];
}

/// A random tree, and a generator of journaled calls to make on it with
/// one builder: walks and updates as in [`scenario`], and structural
/// walks, which are most of them.
///
/// Like [`scenario`], the calls crowd a **busy row** — the children of
/// a node with at least two, picked again when it leaves the tree — so
/// one builder inserts, removes, moves, splits and merges among the
/// same siblings it re-keys. A structural walk's root carries the
/// command; below it, some walks carry field mutations (most of them
/// channel changes) that apply in the walk's first phase, and some a
/// second command on the target's children. About one command in
/// several is invalid: a move under the target's own subtree, a
/// placement beside a sibling of another key, a split of an element
/// that cannot be cut there, a merge with no equal-key next sibling, a
/// removal of the root, a second command on a node the first removed.
pub struct StructuralScenario {
    /// The tree before any call.
    pub tree: Tree<TestElement>,
    /// How many calls to make.
    pub calls: usize,
    rng: StdRng,
    busy: NodeId,
    /// The nodes of [`Self::tree`]: any other node in the tree was
    /// created by a call.
    initial: FxHashSet<NodeId>,
    /// The parents of nodes earlier calls set out to remove, for a later
    /// removal to take next: a subtree removed after a node below it.
    removed_from: Vec<NodeId>,
}

/// The structural scenario for `seed`: a random tree and one to ten
/// calls.
pub fn structural_scenario(seed: u64) -> StructuralScenario {
    // Offset, so that the trees differ from `scenario`'s for one seed.
    let mut rng = StdRng::seed_from_u64(seed.wrapping_add(0xA4_0000));
    let tree = random_tree(&mut rng);
    let busy = busy_parent(&mut rng, &tree);
    let calls = rng.random_range(1..=10);
    let initial = tree.descendants().collect();
    StructuralScenario {
        tree,
        calls,
        rng,
        busy,
        initial,
        removed_from: Vec::new(),
    }
}

/// A random node of `tree` with at least two children, or the root
/// when there is none.
fn busy_parent(rng: &mut StdRng, tree: &Tree<TestElement>) -> NodeId {
    let parents: Vec<NodeId> = tree
        .descendants()
        .filter(|id| id.children(tree.arena()).nth(1).is_some())
        .collect();
    if parents.is_empty() {
        tree.root
    } else {
        pick(rng, &parents)
    }
}

/// A random member of `nodes`, which is not empty.
fn pick(rng: &mut StdRng, nodes: &[NodeId]) -> NodeId {
    nodes[rng.random_range(0..nodes.len())]
}

impl StructuralScenario {
    /// The next call, generated against `tree` as the calls so far left
    /// it, with what it sets out to do.
    pub fn next_call(&mut self, tree: &Tree<TestElement>) -> (Call, Intent) {
        let rng = &mut self.rng;
        if !tree.contains(self.busy) || rng.random_bool(0.1) {
            self.busy = busy_parent(rng, tree);
        }
        let busy = self.busy;
        let nodes: Vec<NodeId> = tree.descendants().collect();
        let row: Vec<NodeId> = busy.children(tree.arena()).collect();
        match rng.random_range(0..10) {
            0 | 1 => {
                let anchors = if rng.random_bool(0.5) {
                    Anchors::Nodes(vec![busy])
                } else {
                    random_anchors(rng, &nodes)
                };
                let mutators = random_mutator_tree(rng, tree, &anchors, false);
                (Call::Walk { mutators, anchors }, Intent::Field)
            }
            2 | 3 => {
                let pool = if row.is_empty() || rng.random_bool(0.3) {
                    &nodes
                } else {
                    &row
                };
                let mutation = if rng.random_bool(0.7) {
                    TestMutation::SetChannel(rng.random_range(0..CHANNELS))
                } else {
                    random_mutation(rng)
                };
                let node = pick(rng, pool);
                (Call::Update { node, mutation }, Intent::Field)
            }
            _ => {
                let created: Vec<NodeId> = nodes
                    .iter()
                    .copied()
                    .filter(|id| !self.initial.contains(id))
                    .collect();
                let context = Context {
                    tree,
                    busy,
                    nodes: &nodes,
                    row: &row,
                    created: &created,
                };
                structural_call(rng, &context, &mut self.removed_from)
            }
        }
    }
}

/// A structural walk against `tree`: a command at its root, anchored
/// at its target (and now and then a second node), with field mutations
/// or a second command below it now and then. See [`StructuralScenario`].
fn structural_call(
    rng: &mut StdRng,
    context: &Context<'_>,
    removed_from: &mut Vec<NodeId>,
) -> (Call, Intent) {
    let Context {
        tree,
        busy,
        nodes,
        row,
        created,
    } = *context;
    let arena = tree.arena();
    let pool: &[NodeId] = if !row.is_empty() && rng.random_bool(0.6) {
        row
    } else {
        nodes
    };
    let intent = Intent::STRUCTURAL[rng.random_range(0..Intent::STRUCTURAL.len())];
    let (target, op) = match intent {
        Intent::Insert => {
            let target = if rng.random_bool(0.6) {
                busy
            } else {
                pick(rng, nodes)
            };
            let (at, sibling_key) = random_placement(rng, tree, target, None);
            let mut subtree = random_subtree(rng, 0);
            if let Some(key) = sibling_key.filter(|_| rng.random_bool(0.75)) {
                subtree.root.kind = key.kind;
                subtree.root.channel = key.channel;
            }
            (target, StructOp::Insert { subtree, at })
        }
        Intent::Remove => {
            let follow = removed_from
                .pop()
                .filter(|&id| tree.contains(id) && id != tree.root);
            let target = match follow {
                Some(parent) if rng.random_bool(0.5) => parent,
                _ if rng.random_bool(0.02) => tree.root,
                _ => pick(rng, pool),
            };
            removed_from.extend(arena[target].parent());
            (target, StructOp::Remove)
        }
        Intent::Move if created.len() >= 2 && rng.random_bool(0.8) => {
            // Created nodes into created subtrees: a builder that inserts
            // one subtree into another after both entered. Placed last and
            // never under the target's own subtree, so that it succeeds.
            let target = pick(rng, created);
            let outside: Vec<NodeId> = created
                .iter()
                .copied()
                .filter(|&id| !id.ancestors(arena).any(|a| a == target))
                .collect();
            let to = if outside.is_empty() {
                pick(rng, created)
            } else {
                pick(rng, &outside)
            };
            let at = Placement::Last;
            (target, StructOp::Move { to, at })
        }
        Intent::Move => {
            let target = pick(rng, pool);
            let to = match rng.random_range(0..12) {
                0 => pick(rng, &target.descendants(arena).collect::<Vec<_>>()),
                1..=5 => busy,
                _ => pick(rng, nodes),
            };
            let key = arena[target].get().key();
            let (at, _) = random_placement(rng, tree, to, Some(key));
            (target, StructOp::Move { to, at })
        }
        Intent::Split => {
            let splittable: Vec<NodeId> = pool
                .iter()
                .copied()
                .filter(|&id| arena[id].get().value >= 2)
                .collect();
            let target = if !splittable.is_empty() && rng.random_bool(0.85) {
                pick(rng, &splittable)
            } else {
                pick(rng, pool)
            };
            let value = arena[target].get().value;
            let at = if value >= 2 && rng.random_bool(0.85) {
                rng.random_range(1..value)
            } else {
                rng.random_range(-1..=6)
            };
            (target, StructOp::Split { at })
        }
        Intent::Merge | Intent::Field => {
            let mergeable: Vec<NodeId> = pool
                .iter()
                .copied()
                .filter(|&id| {
                    arena[id].next_sibling().is_some_and(|next| {
                        arena[next].get().key() == arena[id].get().key()
                            && arena[next].first_child().is_none()
                    })
                })
                .collect();
            let target = if !mergeable.is_empty() && rng.random_bool(0.85) {
                pick(rng, &mergeable)
            } else {
                pick(rng, pool)
            };
            (target, StructOp::Merge)
        }
    };
    let key = BranchKey::new(arena[target].get().kind, rng.random_range(0..CHANNELS));
    let mut mutators = MutatorTree::new_with(Mutator::new_structure(op, key));
    let root = mutators.root;
    if rng.random_bool(0.4) {
        for _ in 0..rng.random_range(1..=2) {
            let mutation = if rng.random_bool(0.6) {
                TestMutation::SetChannel(rng.random_range(0..CHANNELS))
            } else {
                random_mutation(rng)
            };
            let child = Mutator::new(mutation, random_key(rng));
            mutators.insert_child(root, child, Placement::Last).unwrap();
        }
    }
    if rng.random_bool(0.15) {
        let op = if rng.random_bool(0.8) {
            StructOp::Remove
        } else {
            StructOp::Split { at: 1 }
        };
        let child = Mutator::new_structure(op, random_key(rng));
        mutators.insert_child(root, child, Placement::Last).unwrap();
    }
    let mut targets = vec![target];
    if rng.random_bool(0.15) {
        targets.push(pick(rng, pool));
    }
    let anchors = Anchors::Nodes(targets);
    (Call::Walk { mutators, anchors }, intent)
}

/// What a structural call is generated against: the tree as the calls
/// so far left it, the busy row's parent and its children, every node
/// in the tree, and the nodes calls created.
#[derive(Clone, Copy)]
struct Context<'a> {
    tree: &'a Tree<TestElement>,
    busy: NodeId,
    nodes: &'a [NodeId],
    row: &'a [NodeId],
    created: &'a [NodeId],
}

/// A random placement among `parent`'s children: `Last` or `First`
/// most of the time, otherwise `After` or `Before` a child — one keyed
/// `key`, when there is one, three times in four — with that child's
/// key, which a placement next to it needs.
fn random_placement(
    rng: &mut StdRng,
    tree: &Tree<TestElement>,
    parent: NodeId,
    key: Option<BranchKey<TestKind>>,
) -> (Placement, Option<BranchKey<TestKind>>) {
    let children: Vec<NodeId> = parent.children(tree.arena()).collect();
    let roll = rng.random_range(0..10);
    if roll <= 3 || children.is_empty() {
        return (Placement::Last, None);
    }
    if roll <= 6 {
        return (Placement::First, None);
    }
    let keyed: Vec<NodeId> = match key {
        Some(key) => children
            .iter()
            .copied()
            .filter(|&id| tree.arena()[id].get().key() == key)
            .collect(),
        None => Vec::new(),
    };
    let sibling = if !keyed.is_empty() && rng.random_bool(0.75) {
        pick(rng, &keyed)
    } else {
        pick(rng, &children)
    };
    let sibling_key = tree.arena()[sibling].get().key();
    let at = if roll <= 8 {
        Placement::After(sibling)
    } else {
        Placement::Before(sibling)
    };
    (at, Some(sibling_key))
}

/// A random subtree of up to three levels and two children a node.
fn random_subtree(rng: &mut StdRng, depth: usize) -> Subtree<TestElement> {
    let mut subtree = Subtree::leaf(random_element(rng));
    if depth < 2 {
        let count = rng.random_range(0..=2);
        subtree.children = (0..count).map(|_| random_subtree(rng, depth + 1)).collect();
    }
    subtree
}

/// What a call changed, read from snapshots of the tree before and
/// after it — never from a journal.
#[derive(Clone, Copy, Debug, Default)]
pub struct Effect {
    /// A node entered or left the tree, or changed parent.
    pub structural: bool,
    /// A node changed parent.
    pub reparented: bool,
    /// A node kept its parent but changed its channel and its previous
    /// sibling: a key move.
    pub rekeyed: bool,
}

/// Each node of a snapshot with its parent, its previous sibling and
/// its element.
pub fn placed(rows: &[SnapshotRow]) -> FxHashMap<NodeId, (Option<NodeId>, Option<NodeId>, TestElement)> {
    let mut last_child: FxHashMap<Option<NodeId>, NodeId> = FxHashMap::default();
    rows.iter()
        .map(|(id, parent, element)| {
            let prev = last_child.insert(*parent, *id);
            (*id, (*parent, prev, element.clone()))
        })
        .collect()
}

/// What changed between the snapshots `pre` and `post`.
pub fn effect(pre: &[SnapshotRow], post: &[SnapshotRow]) -> Effect {
    let (pre, post) = (placed(pre), placed(post));
    let mut effect = Effect {
        structural: pre.len() != post.len() || pre.keys().any(|id| !post.contains_key(id)),
        ..Effect::default()
    };
    for (id, (parent, prev, element)) in &pre {
        let Some((post_parent, post_prev, post_element)) = post.get(id) else {
            continue;
        };
        if parent != post_parent {
            effect.structural = true;
            effect.reparented = true;
        } else if element.channel != post_element.channel && prev != post_prev {
            effect.rekeyed = true;
        }
    }
    effect
}

/// `node`'s ancestors in a snapshot, nearest first.
fn ancestors(
    rows: &FxHashMap<NodeId, (Option<NodeId>, Option<NodeId>, TestElement)>,
    node: NodeId,
) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut cursor = rows.get(&node).and_then(|(parent, _, _)| *parent);
    while let Some(id) = cursor {
        out.push(id);
        cursor = rows.get(&id).and_then(|(parent, _, _)| *parent);
    }
    out
}

/// The name of an error's variant, for counting.
pub fn error_name(error: &TreeError) -> &'static str {
    match error {
        TreeError::StaleNode(_) => "StaleNode",
        TreeError::PlacementCrossesKey => "PlacementCrossesKey",
        TreeError::NotAChild { .. } => "NotAChild",
        TreeError::RemoveRoot => "RemoveRoot",
        TreeError::Unsorted { .. } => "Unsorted",
        TreeError::Cycle { .. } => "Cycle",
        TreeError::Unsplittable(_) => "Unsplittable",
        TreeError::Unmergeable(_) => "Unmergeable",
        TreeError::TargetRemovedEarlier(_) => "TargetRemovedEarlier",
    }
}

/// What the structural runs did, counted from the tree, for the
/// properties' preconditions.
#[derive(Debug, Default)]
pub struct StructuralStats {
    /// Calls that succeeded and changed structure, by what they set out
    /// to do.
    pub structural: FxHashMap<Intent, u64>,
    /// Successful calls that moved a node to another parent.
    pub reparented: u64,
    /// Failed calls, by error.
    pub errors: FxHashMap<&'static str, u64>,
    /// Builders that, when they closed, held a call that changed
    /// structure and **another** call that moved a node by its key.
    pub mixed: u64,
    /// Builders that closed holding a call made after one of their
    /// calls had failed and unwound them.
    pub continued_after_failure: u64,
    /// Runs whose tree changed.
    pub changed: u64,
    /// Builders that closed holding a removal of a node above one an
    /// earlier call of theirs had removed: a subtree that left in pieces.
    pub nested_removals: u64,
    /// Builders that closed holding a move of a node they had created
    /// into another node they had created: a subtree that entered in
    /// pieces.
    pub created_into_created: u64,
}

/// One structural run: the tree as its calls left it, snapshots from
/// before the first call and after the last, and the closed journal.
pub struct StructuralRun {
    /// The tree after the calls.
    pub tree: Tree<TestElement>,
    /// The snapshot before the first call.
    pub before: Vec<SnapshotRow>,
    /// The snapshot after the last call.
    pub after: Vec<SnapshotRow>,
    /// The builder's journal.
    pub journal: Journal<TestElement>,
}

/// Run the structural scenario for `seed` on one builder, counting into
/// `stats`, and check what every call must leave behind: after a
/// success, sorted rows; after a failure, the tree exactly as the
/// builder found it, every node the builder created freed, and the
/// builder empty — which the following calls then use again.
pub fn run_structural(seed: u64, stats: &mut StructuralStats) -> StructuralRun {
    let mut scenario = structural_scenario(seed);
    let mut tree = scenario.tree.clone();
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();
    let mut effects: Vec<Effect> = Vec::new();
    let mut failed = false;
    let start = placed(&before);
    let mut removed: FxHashSet<NodeId> = FxHashSet::default();
    let (mut nested_removal, mut created_into_created) = (false, false);
    for _ in 0..scenario.calls {
        let (call, intent) = scenario.next_call(&tree);
        let pre = snapshot(&tree);
        match call.run(&mut tree, &mut builder) {
            Ok(()) => {
                tree.check_sorted()
                    .unwrap_or_else(|e| panic!("seed {seed}: rows are sorted after a {intent:?} call: {e}"));
                let post = snapshot(&tree);
                let (pre_placed, post_placed) = (placed(&pre), placed(&post));
                for id in pre_placed.keys().filter(|id| !post_placed.contains_key(id)) {
                    let mut ancestry = removed.iter().map(|&earlier| ancestors(&start, earlier));
                    nested_removal |= ancestry.any(|chain| chain.contains(id));
                    removed.insert(*id);
                }
                created_into_created |= post_placed.iter().any(|(id, (parent, _, _))| {
                    let moved = pre_placed.get(id).is_some_and(|(was, _, _)| was != parent);
                    moved
                        && !start.contains_key(id)
                        && parent.is_some_and(|parent| !start.contains_key(&parent))
                });
                let effect = effect(&pre, &post);
                if effect.structural {
                    *stats.structural.entry(intent).or_default() += 1;
                }
                if effect.reparented {
                    stats.reparented += 1;
                }
                effects.push(effect);
            }
            Err(e) => {
                assert_eq!(
                    snapshot(&tree),
                    before,
                    "seed {seed}: a failing {intent:?} call ({e}) leaves the tree as the builder found it"
                );
                assert_eq!(builder.recorded_len(), 0, "seed {seed}: and the builder empty");
                let existed: FxHashSet<NodeId> = before.iter().map(|(id, _, _)| *id).collect();
                for (id, _, _) in &pre {
                    if !existed.contains(id) {
                        assert!(
                            tree.arena().get(*id).is_none(),
                            "seed {seed}: node {id}, which the builder created, is freed"
                        );
                    }
                }
                *stats.errors.entry(error_name(&e)).or_default() += 1;
                effects.clear();
                removed.clear();
                (nested_removal, created_into_created) = (false, false);
                failed = true;
            }
        }
    }
    let structural = effects.iter().filter(|e| e.structural).count();
    let rekeyed = effects.iter().filter(|e| e.rekeyed).count();
    let both = effects.iter().filter(|e| e.structural && e.rekeyed).count();
    if structural > 0 && rekeyed > 0 && (structural > both || rekeyed > both || both >= 2) {
        stats.mixed += 1;
    }
    if failed && !effects.is_empty() {
        stats.continued_after_failure += 1;
    }
    stats.nested_removals += u64::from(nested_removal);
    stats.created_into_created += u64::from(created_into_created);
    let after = snapshot(&tree);
    if before != after {
        stats.changed += 1;
    }
    let journal = builder.close(&tree);
    StructuralRun {
        tree,
        before,
        after,
        journal,
    }
}
