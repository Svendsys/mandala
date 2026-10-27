// SPDX-License-Identifier: MPL-2.0

//! Anchored walks: [`MutatorTree::apply_at`](crate::engine::tree::MutatorTree::apply_at)
//! starts a walk at each of a set of [`Anchors`](crate::engine::anchor::Anchors) —
//! typically the selection — instead of at the tree's root, and reaches
//! only the anchored subtrees.
//!
//! Anchors are resolved to concrete nodes before anything is walked:
//! relative forms ([`Anchors::ParentOf`](crate::engine::anchor::Anchors::ParentOf),
//! [`Anchors::SiblingsOf`](crate::engine::anchor::Anchors::SiblingsOf)) become the nodes
//! they name, de-duplicated. A mutator tree's shape is relative to its
//! root, so only the nodes of the root's **kind** stay anchors — at a
//! node of another kind the mutator does not apply, and nothing below
//! it is reached. Among those, when the mutator reaches below its root,
//! every anchor inside another anchor's subtree is dropped, so a
//! subtree mutation reaches each element exactly once however the
//! selection nests. What remains is walked in tree order.
//!
//! The work is proportional to the anchors and the paths above them,
//! never to the rest of the tree: no anchor is resolved, positioned or
//! seeded twice, no element above an anchor is climbed through twice —
//! however many anchors sit below it, so a selection of every element
//! of a deep chain costs the chain once, not once per element (§20
//! rule 11) — a sibling row is scanned at most once to resolve
//! `SiblingsOf` and at most once to order two anchors whose keys tie,
//! and siblings of different keys are ordered by their keys alone — the
//! sorted-children invariant.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::journal::JournalBuilder;
use crate::engine::membership::Membership;
use crate::engine::mutation::Mutation;
use crate::engine::tree::{MutatorTree, Tree, TreeError};
use crate::engine::walker::{run_walk, walk_anchor};
use indextree::{Arena, NodeId};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::cmp::Ordering;

/// Where an anchored walk starts.
///
/// Each form resolves to a concrete set of nodes before the walk; the
/// same node named twice, or reached through two of the given nodes,
/// is one anchor, and the anchors are walked in tree order (pre-order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anchors {
    /// The nodes themselves.
    Nodes(Vec<NodeId>),
    /// The distinct parents of the nodes. The root has no parent and
    /// contributes nothing.
    ParentOf(Vec<NodeId>),
    /// For each node, its siblings of the same
    /// [kind](crate::engine::element::TreeElement::kind), excluding the
    /// node itself. The root and an only child contribute nothing.
    SiblingsOf(Vec<NodeId>),
}

impl Anchors {
    /// The nodes this anchor set is stated in terms of — every one of
    /// them must be in the tree for the walk to run. O(1).
    fn given(&self) -> &[NodeId] {
        match self {
            Anchors::Nodes(nodes) | Anchors::ParentOf(nodes) | Anchors::SiblingsOf(nodes) => nodes,
        }
    }
}

/// The body of [`MutatorTree::apply_at`] and
/// [`MutatorTree::apply_at_journaled`]: validate, resolve to the root's
/// kind, drop nested anchors, order, seed every anchor, then walk each
/// one and run the structural commands collected, recording into
/// `journal` when there is one. See `apply_at` for the contract and its
/// cost.
pub(in crate::engine) fn apply_anchored<E, M, F>(
    mutator_tree: &MutatorTree<E, M>,
    tree: &mut Tree<E>,
    anchors: &Anchors,
    seed_for: F,
    journal: Option<&mut JournalBuilder<E>>,
) -> Result<(), TreeError>
where
    E: TreeElement,
    M: Mutation<E>,
    F: Fn(&Tree<E>, NodeId) -> E::Inherited,
{
    // All or nothing: every node the anchors are stated in terms of is
    // checked — in the arena, and under this tree's root — before
    // anything resolves, let alone walks. A node named twice is checked
    // once, and each ancestor is climbed through once over all of them.
    let mut members = Membership::default();
    let mut given: FxHashSet<NodeId> = FxHashSet::default();
    for &id in anchors.given() {
        if given.insert(id) && !members.contains(tree, id) {
            return Err(TreeError::StaleNode(id));
        }
    }

    // The root's kind decides which nodes are anchors at all (no kind
    // match, no walk); nesting is then decided among those alone, so an
    // anchor of the root's kind is never dropped for an ancestor the
    // root would not apply at.
    let root_kind = mutator_tree.arena[mutator_tree.root].get().key().kind;
    let mut resolved = resolve(&tree.arena, anchors, &given, root_kind);

    if resolved.len() > 1 {
        resolved = in_tree_order(tree, &mut members, &resolved, mutator_tree.reaches_below_root());
    }

    // Every seed is taken from the tree as it is now, before any
    // anchor's walk can change an ancestor another anchor inherits
    // from — and only for anchors that will be walked.
    let seeded: Vec<(NodeId, E::Inherited)> = resolved
        .into_iter()
        .map(|id| {
            let seed = seed_for(tree, id);
            (id, seed)
        })
        .collect();

    let result = run_walk(tree, mutator_tree, journal, |tree, walk| {
        for (anchor, seed) in &seeded {
            walk_anchor(tree, mutator_tree, *anchor, seed, walk);
        }
    });
    tree.cache.invalidate();
    result
}

/// The distinct nodes `anchors` names that have the kind `root_kind`,
/// in no particular order. Every given node is in the tree (the caller
/// checked) and `given` holds them without repeats.
///
/// Cost: O(given) for `Nodes` and `ParentOf`. `SiblingsOf` scans the
/// row of each distinct parent of the given nodes once — the row is
/// what it resolves to.
fn resolve<E: TreeElement>(
    arena: &Arena<E>,
    anchors: &Anchors,
    given: &FxHashSet<NodeId>,
    root_kind: E::Kind,
) -> Vec<NodeId> {
    let of_root_kind = |id: &NodeId| arena[*id].get().kind() == root_kind;
    match anchors {
        Anchors::Nodes(_) => given.iter().copied().filter(of_root_kind).collect(),
        Anchors::ParentOf(_) => {
            let parents: FxHashSet<NodeId> = given.iter().filter_map(|&id| arena[id].parent()).collect();
            parents.into_iter().filter(of_root_kind).collect()
        }
        Anchors::SiblingsOf(_) => {
            // A child of the root's kind is a sibling of some given node
            // when a given node of that same kind, other than the child
            // itself, shares its parent. Only the given nodes of the
            // root's kind can name such a child, so count those per
            // parent.
            let mut given_per_parent: FxHashMap<NodeId, usize> = FxHashMap::default();
            for &id in given {
                if let Some(parent) = arena[id].parent() {
                    let count = given_per_parent.entry(parent).or_insert(0);
                    if of_root_kind(&id) {
                        *count += 1;
                    }
                }
            }
            let mut out = Vec::new();
            for (parent, naming) in given_per_parent {
                if naming == 0 {
                    continue;
                }
                out.extend(
                    parent
                        .children(arena)
                        .filter(|child| of_root_kind(child) && naming > usize::from(given.contains(child))),
                );
            }
            out
        }
    }
}

/// `anchors` — distinct, every one in the tree — in tree order
/// (pre-order), without the ones nested in another when
/// `drop_nested`: an anchor with another anchor among its ancestors is
/// reached through that one.
///
/// Nothing climbs from an anchor to the root more than once over all of
/// them. Each anchor is climbed through `members`, whose climbs stop at
/// the first element already climbed through, so afterwards its members
/// are exactly the union of the anchors' paths to the root — each
/// element of it climbed through once (§20 rule 11). That union is then
/// a small tree: each member hangs under its parent, the members under
/// one parent are ordered as their row has them — by key, the
/// sorted-children invariant, and between equal keys by position, from
/// one scan of that row the first time a tie needs it — and one
/// pre-order pass over it from the root lists the anchors, not
/// descending below an anchor when nested ones are dropped.
///
/// Cost, with `u` the size of that union: O(u log u) for the order,
/// one hash map and two `Vec`s of `u` entries, plus the row scans
/// ties need. Nothing is proportional to the depth per anchor.
fn in_tree_order<E: TreeElement>(
    tree: &Tree<E>,
    members: &mut Membership,
    anchors: &[NodeId],
    drop_nested: bool,
) -> Vec<NodeId> {
    // A resolved anchor is a given node, its parent or its sibling, so
    // it is in the tree; the climb only adds its path to the union.
    for &id in anchors {
        members.contains(tree, id);
    }
    let arena = tree.arena();
    let mut edges: Vec<(NodeId, NodeId)> = members
        .members()
        .filter_map(|id| arena[id].parent().map(|parent| (parent, id)))
        .collect();
    let ties = TiePositions::default();
    edges.sort_unstable_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| sibling_order(arena, &ties, a.0, a.1, b.1))
    });
    // Each parent's members are one run of `edges`.
    let mut runs: FxHashMap<NodeId, (usize, usize)> = FxHashMap::default();
    let mut start = 0;
    while let Some(&(parent, _)) = edges.get(start) {
        let len = edges[start..].iter().take_while(|edge| edge.0 == parent).count();
        runs.insert(parent, (start, start + len));
        start += len;
    }
    let is_anchor: FxHashSet<NodeId> = anchors.iter().copied().collect();
    let mut ordered = Vec::with_capacity(anchors.len());
    let mut pending = vec![tree.root()];
    while let Some(node) = pending.pop() {
        if is_anchor.contains(&node) {
            ordered.push(node);
            if drop_nested {
                continue;
            }
        }
        if let Some(&(from, to)) = runs.get(&node) {
            pending.extend(edges[from..to].iter().rev().map(|&(_, child)| child));
        }
    }
    ordered
}

/// Sibling positions within the parent rows where two anchors' paths
/// part on siblings with equal keys — the only case keys cannot order.
/// Each such row is scanned once, the first time it is needed.
#[derive(Default)]
struct TiePositions {
    scanned: RefCell<FxHashSet<NodeId>>,
    position: RefCell<FxHashMap<NodeId, usize>>,
}

impl TiePositions {
    /// `child`'s index among the children of `parent`.
    fn of<E>(&self, arena: &Arena<E>, parent: NodeId, child: NodeId) -> usize {
        if self.scanned.borrow_mut().insert(parent) {
            let mut position = self.position.borrow_mut();
            for (index, sibling) in parent.children(arena).enumerate() {
                position.insert(sibling, index);
            }
        }
        self.position.borrow().get(&child).copied().unwrap_or(0)
    }
}

/// The order of `x` and `y`, two children of `parent`, in its row: by
/// key — the sorted-children invariant — and between equal keys by
/// position, from the row's one-time scan.
fn sibling_order<E: TreeElement>(
    arena: &Arena<E>,
    ties: &TiePositions,
    parent: NodeId,
    x: NodeId,
    y: NodeId,
) -> Ordering {
    let by_key = arena[x].get().key().cmp(&arena[y].get().key());
    if by_key != Ordering::Equal {
        return by_key;
    }
    ties.of(arena, parent, x).cmp(&ties.of(arena, parent, y))
}
