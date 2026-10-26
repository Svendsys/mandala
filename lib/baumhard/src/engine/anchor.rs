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
//! The work is proportional to the anchors, never to the tree: no
//! anchor is resolved, positioned or seeded twice, a sibling row is
//! scanned at most once to resolve `SiblingsOf` and at most once to
//! order two anchors whose keys tie, and siblings of different keys are
//! ordered by their keys alone — the sorted-children invariant.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::journal::JournalBuilder;
use crate::engine::mutation::Mutation;
use crate::engine::tree::{MutatorTree, Tree, TreeError};
use crate::engine::walker::{walk_anchor, Walk};
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
/// one, recording into `journal` when there is one. See `apply_at` for
/// the contract and its cost.
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
    // once.
    let mut given: FxHashSet<NodeId> = FxHashSet::default();
    for &id in anchors.given() {
        if given.insert(id) && !is_in_tree(&tree.arena, tree.root, id) {
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
        let mut placed: Vec<Placed> = resolved
            .iter()
            .map(|&id| Placed {
                id,
                path: path_from_root(&tree.arena, id),
            })
            .collect();
        if mutator_tree.reaches_below_root() {
            let anchor_set: FxHashSet<NodeId> = resolved.iter().copied().collect();
            // `path` ends at the anchor itself; an anchor with another
            // anchor among its ancestors is reached through that one.
            placed.retain(|p| !p.path[..p.path.len() - 1].iter().any(|a| anchor_set.contains(a)));
        }
        let ties = TiePositions::default();
        placed.sort_by(|a, b| tree_order(&tree.arena, &ties, a, b));
        resolved = placed.into_iter().map(|p| p.id).collect();
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

    let mut walk = Walk::new(journal);
    for (anchor, seed) in &seeded {
        walk_anchor(tree, mutator_tree, *anchor, seed, &mut walk);
    }
    walk.settle(tree);
    tree.cache.invalidate();
    Ok(())
}

/// Whether `id` is a node of the arena that has `root` among its
/// ancestors (or is `root`). A removed node, a reused slot under an old
/// stamp, and a node detached from the tree all answer `false`.
/// O(depth of `id`).
fn is_in_tree<E>(arena: &Arena<E>, root: NodeId, id: NodeId) -> bool {
    if arena.get(id).is_none() {
        return false;
    }
    id.ancestors(arena).last() == Some(root)
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

/// An anchor and the path to it from the root, root first and the
/// anchor last.
struct Placed {
    id: NodeId,
    path: Vec<NodeId>,
}

/// `id`'s ancestors from the root down to `id` itself. O(depth); one
/// `Vec` of that length.
fn path_from_root<E>(arena: &Arena<E>, id: NodeId) -> Vec<NodeId> {
    let mut path: Vec<NodeId> = id.ancestors(arena).collect();
    path.reverse();
    path
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

/// Pre-order of two anchors. Their paths share the root; where they
/// part, the two nodes are siblings, ordered by key — the sorted-children
/// invariant — and, between equal keys, by position in their row. A
/// path that is a prefix of the other is an ancestor and comes first.
/// O(depth) plus, on a tie, the row's one-time scan.
fn tree_order<E: TreeElement>(arena: &Arena<E>, ties: &TiePositions, a: &Placed, b: &Placed) -> Ordering {
    let shared = a.path.iter().zip(&b.path).take_while(|(x, y)| x == y).count();
    match (a.path.get(shared), b.path.get(shared)) {
        (Some(&x), Some(&y)) => {
            let by_key = arena[x].get().key().cmp(&arena[y].get().key());
            if by_key != Ordering::Equal {
                return by_key;
            }
            // Every path starts at the tree's root — each anchor was
            // validated as under it — so two paths share at least that
            // first node, and the nodes where they part have a parent.
            debug_assert!(shared > 0, "anchor paths share the root");
            let parent = a.path[shared - 1];
            ties.of(arena, parent, x).cmp(&ties.of(arena, parent, y))
        }
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
