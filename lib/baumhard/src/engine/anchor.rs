// SPDX-License-Identifier: MPL-2.0

//! Anchored walks: [`MutatorTree::apply_at`](crate::engine::tree::MutatorTree::apply_at)
//! starts a walk at each of a set of [`Anchors`](crate::engine::anchor::Anchors) —
//! typically the selection — instead of at the tree's root, and reaches
//! only the anchored subtrees.
//!
//! Anchors are resolved to concrete nodes before anything is walked:
//! relative forms ([`Anchors::ParentOf`](crate::engine::anchor::Anchors::ParentOf),
//! [`Anchors::SiblingsOf`](crate::engine::anchor::Anchors::SiblingsOf)) become the nodes
//! they name, the list is put in tree order and de-duplicated, and —
//! when the mutator reaches below its root — every anchor inside
//! another anchor's subtree is dropped, so a subtree mutation reaches
//! each element exactly once however the selection nests.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::mutation::Mutation;
use crate::engine::tree::{MutatorTree, Tree, TreeError};
use crate::engine::walker::{walk_anchor, Rekeyed};
use indextree::{Arena, NodeId};

/// Where an anchored walk starts.
///
/// Each form resolves to a concrete list of nodes before the walk; the
/// list is put in tree order (pre-order) and de-duplicated, so the
/// same node named twice, or reached through two of the given nodes,
/// is one anchor.
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

/// The body of [`MutatorTree::apply_at`]: validate, resolve, order,
/// drop nested anchors, seed every anchor, then walk each one. See that
/// method for the contract.
pub(crate) fn apply_anchored<E, M, F>(
    mutator_tree: &MutatorTree<E, M>,
    tree: &mut Tree<E>,
    anchors: &Anchors,
    seed_for: F,
) -> Result<(), TreeError>
where
    E: TreeElement,
    M: Mutation<E>,
    F: Fn(&Tree<E>, NodeId) -> E::Inherited,
{
    // All or nothing: every node the anchors are stated in terms of is
    // checked before anything resolves, let alone walks.
    for &id in anchors.given() {
        if tree.arena.get(id).is_none() {
            return Err(TreeError::StaleNode(id));
        }
    }

    let resolved = resolve(&tree.arena, anchors);
    let mut ordered = Vec::with_capacity(resolved.len());
    for id in resolved {
        ordered.push((tree_position(&tree.arena, tree.root, id)?, id));
    }
    ordered.sort_unstable();
    ordered.dedup_by(|later, earlier| later.1 == earlier.1);

    if mutator_tree.reaches_below_root() {
        // Pre-order puts a node's whole subtree right after it, so an
        // anchor inside another anchor's subtree is inside the subtree
        // of the last anchor kept before it.
        let mut kept_path: Option<Vec<usize>> = None;
        ordered.retain(|(path, _)| {
            if kept_path.as_ref().is_some_and(|kept| path.starts_with(kept)) {
                return false;
            }
            kept_path = Some(path.clone());
            true
        });
    }

    // Every seed is taken from the tree as it is now, before any
    // anchor's walk can change an ancestor another anchor inherits
    // from.
    let seeded: Vec<(NodeId, E::Inherited)> = ordered
        .into_iter()
        .map(|(_, id)| {
            let seed = seed_for(tree, id);
            (id, seed)
        })
        .collect();

    let mut rekeyed = Rekeyed::default();
    for (anchor, seed) in &seeded {
        walk_anchor(tree, mutator_tree, *anchor, seed, &mut rekeyed);
    }
    rekeyed.settle(tree);
    tree.cache.invalidate();
    Ok(())
}

/// The concrete nodes `anchors` names, in no particular order and
/// possibly repeated. Every given node is in the arena (the caller
/// checked). O(given nodes), plus the sibling rows for
/// [`Anchors::SiblingsOf`].
fn resolve<E: TreeElement>(arena: &Arena<E>, anchors: &Anchors) -> Vec<NodeId> {
    match anchors {
        Anchors::Nodes(nodes) => nodes.clone(),
        Anchors::ParentOf(nodes) => nodes
            .iter()
            .filter_map(|&id| arena.get(id).and_then(|n| n.parent()))
            .collect(),
        Anchors::SiblingsOf(nodes) => {
            let mut out = Vec::new();
            for &id in nodes {
                let Some(node) = arena.get(id) else {
                    continue;
                };
                let Some(parent) = node.parent() else {
                    continue;
                };
                let kind = node.get().kind();
                out.extend(
                    parent
                        .children(arena)
                        .filter(|&sibling| sibling != id && arena[sibling].get().kind() == kind),
                );
            }
            out
        }
    }
}

/// `id`'s position in pre-order, as the path of sibling indices from
/// the root down: `[]` for the root, `[0, 2]` for the third child of
/// the root's first child. Paths compare lexicographically in exactly
/// pre-order — an ancestor's path is a prefix of, and sorts before,
/// every path in its subtree — and two nodes share a path only if they
/// are one node.
///
/// [`TreeError::StaleNode`] if `id` is not in the tree under `root`.
///
/// Cost: O(depth + the siblings before each ancestor); one `Vec` of
/// the node's depth.
fn tree_position<E>(arena: &Arena<E>, root: NodeId, id: NodeId) -> Result<Vec<usize>, TreeError> {
    let mut path = Vec::new();
    let mut cursor = id;
    while cursor != root {
        let Some(node) = arena.get(cursor) else {
            return Err(TreeError::StaleNode(id));
        };
        let Some(parent) = node.parent() else {
            // Reached a top without meeting the root: not in this tree.
            return Err(TreeError::StaleNode(id));
        };
        path.push(cursor.preceding_siblings(arena).skip(1).count());
        cursor = parent;
    }
    path.reverse();
    Ok(path)
}
