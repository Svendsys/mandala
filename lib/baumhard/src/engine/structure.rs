// SPDX-License-Identifier: MPL-2.0

//! Structural commands: a mutator that changes the **shape** of a tree —
//! inserts a subtree, removes one, moves one under another parent, cuts
//! an element in two or joins two into one — rather than a field of an
//! element.
//!
//! A command is a [`StructOp`](crate::engine::structure::StructOp) carried
//! by a [`Mutator::Structure`](crate::engine::mutator::Mutator::Structure)
//! node, and it reaches its target the way every mutator does: by key,
//! from the walk's anchors. What differs is when it runs.
//!
//! # Two phases
//!
//! 1. **The walk** collects every `(target, command)` pair it reaches,
//!    in the order it reaches them — pre-order — and changes no
//!    structure. The field mutations of the same mutator tree apply
//!    during the walk as always, and the walk's re-keyed children are
//!    settled into their sorted places when it ends. So the walk never
//!    iterates a sibling row a command is changing.
//! 2. **The commands** then run in that order, each one recorded in the
//!    walk's journal. If one fails, the whole call is undone through
//!    the journal — the walk's field changes and the commands before it
//!    included — and its error is returned.
//!
//! Every command keeps the sorted-children invariant: a node goes in at
//! a [`Placement`](crate::engine::tree::Placement) within the run of its
//! own key.
//!
//! # Detached, not freed
//!
//! A removed subtree is **detached**: it leaves the tree but stays in
//! the arena, owned by the journal that recorded the removal, so undo
//! puts back the same `NodeId`s and every id held elsewhere — a
//! selection, a reference, another journal — stays valid. Undoing an
//! insert detaches the inserted subtree the same way, so redo puts back
//! the same ids too. Nothing here frees a node; a detached subtree is
//! freed by [`Journal::discard`](crate::engine::journal::Journal::discard)
//! once its journal leaves the history.
//!
//! The engine knows nothing about what a structure *means*: a node
//! moved under its own descendant is the generic
//! [`Cycle`](crate::engine::tree::TreeError::Cycle), and any rule about
//! which element may hold which is the instantiation's to check.
//! Resolving a persistent id to the `NodeId` a
//! [`StructOp::Move`](crate::engine::structure::StructOp::Move) names is
//! the instantiation's too.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::journal::JournalBuilder;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::engine::siblings;
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use indextree::{Arena, NodeId};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

/// A tree of elements that is not in any [`Tree`] yet — what a
/// [`StructOp::Insert`] puts in. `children` are in any order: each is
/// placed by its key, at the end of its run, so equal keys keep the
/// order given.
///
/// Serializable when the element is; nothing requires it to be.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subtree<E> {
    /// The element at the top: the inserted node.
    pub root: E,
    /// The subtrees below it.
    pub children: Vec<Subtree<E>>,
}

impl<E> Subtree<E> {
    /// A subtree of one element. No allocation.
    pub fn leaf(root: E) -> Self {
        Subtree {
            root,
            children: Vec::new(),
        }
    }
}

/// A structural command: what a [`Mutator::Structure`] does to the
/// element it reaches — its **target**.
///
/// Every command records what it changes in the walk's journal, so
/// [`Journal::revert`](crate::engine::journal::Journal::revert) and
/// [`Journal::replay`](crate::engine::journal::Journal::replay) undo and
/// redo it exactly, with the same `NodeId`s.
///
/// Errors common to every command: [`TreeError::TargetRemovedEarlier`]
/// for a target an earlier command of the same walk removed.
#[derive(Clone, Debug)]
pub enum StructOp<E: TreeElement> {
    /// Insert `subtree` as a child of the target, at `at` within the
    /// run of children whose key equals the subtree root's. Every
    /// inserted node is new.
    ///
    /// Errors: those of a placement — [`TreeError::StaleNode`],
    /// [`TreeError::NotAChild`] or [`TreeError::PlacementCrossesKey`]
    /// for a `Before` / `After` sibling that is gone, under another
    /// parent or keyed otherwise.
    ///
    /// Cost: one arena slot per subtree element, plus a scan of the
    /// target's row for the placement.
    Insert {
        /// What to insert.
        subtree: Subtree<E>,
        /// Where among the target's children.
        at: Placement,
    },
    /// Remove the target and its subtree. The subtree is detached, not
    /// freed (see the module docs).
    ///
    /// Errors: [`TreeError::RemoveRoot`] for the tree's root.
    ///
    /// Cost: O(1) beside the journal's record of the target.
    Remove,
    /// Move the target, with its subtree, under `to`, at `at` within the
    /// run of `to`'s children whose key equals the target's. `to` may
    /// be the target's own parent, which moves it within its row.
    ///
    /// Errors: [`TreeError::Cycle`] when `to` is the target or in its
    /// subtree (the root included: everything is in its subtree);
    /// [`TreeError::StaleNode`] for a `to` not in the tree, or
    /// [`TreeError::TargetRemovedEarlier`] for one an earlier command
    /// of the walk removed; those of a placement, as for
    /// [`Self::Insert`] — a `Before` / `After` naming the target itself
    /// is [`TreeError::NotAChild`].
    ///
    /// Cost: O(depth of `to`) to check for a cycle, plus a scan of
    /// `to`'s row for the placement.
    Move {
        /// The new parent.
        to: NodeId,
        /// Where among its children.
        at: Placement,
    },
    /// Cut the target in two at `at` ([`TreeElement::split`]): the
    /// target keeps the first part and its children, and the second is
    /// inserted right after it as a new childless node.
    ///
    /// Errors: [`TreeError::Unsplittable`] when the element cannot be
    /// cut there or the target is the root;
    /// [`TreeError::PlacementCrossesKey`] when a part's key is not the
    /// target's.
    ///
    /// Cost: the element's `split`, and one arena slot.
    Split {
        /// Where to cut.
        at: E::SplitPoint,
    },
    /// The target absorbs its next sibling ([`TreeElement::merge`]),
    /// which is removed — detached, as by [`Self::Remove`].
    ///
    /// Errors: [`TreeError::Unmergeable`] when the target has no next
    /// sibling, the next sibling's key differs or it has children, or
    /// the element cannot absorb it; [`TreeError::PlacementCrossesKey`]
    /// when the merged element's key is not the target's.
    ///
    /// Cost: the element's `merge`.
    Merge,
}

/// Run the commands a walk collected, in order: `ops` holds each
/// target with the id of the [`Mutator::Structure`] node in
/// `mutator_tree` that reached it. Every change is recorded in
/// `journal`. On an error the commands before it have run; the caller
/// undoes them, with the rest of its call, through `journal`.
///
/// Invalidates `tree`'s cache. Cost: each command's, plus a hash set
/// over the subtrees the commands remove, allocated only when one does.
pub(in crate::engine) fn apply_ops<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    ops: &[(NodeId, NodeId)],
    journal: &mut JournalBuilder<E>,
) -> Result<(), TreeError> {
    let mut removed: FxHashSet<NodeId> = FxHashSet::default();
    let result = ops.iter().try_for_each(|&(target, mutator)| {
        match mutator_tree.arena.get(mutator).map(|node| node.get()) {
            Some(Mutator::Structure { op, .. }) => apply_op(tree, target, op, journal, &mut removed),
            _ => Ok(()),
        }
    });
    tree.cache.invalidate();
    result
}

/// Run one command on `target`. `removed` holds the roots of the
/// subtrees this walk's earlier commands detached.
fn apply_op<E: TreeElement>(
    tree: &mut Tree<E>,
    target: NodeId,
    op: &StructOp<E>,
    journal: &mut JournalBuilder<E>,
    removed: &mut FxHashSet<NodeId>,
) -> Result<(), TreeError> {
    let root = tree.root;
    let arena = &mut tree.arena;
    in_tree(arena, root, target, removed)?;
    match op {
        StructOp::Insert { subtree, at } => {
            let prev = siblings::slot(arena, target, &subtree.root.key(), *at, E::key)?;
            let inserted = build_detached(arena, subtree);
            journal.record_created(arena, inserted);
            journal.note_attach(arena, target, prev);
            siblings::attach_at(arena, target, inserted, prev);
        }
        StructOp::Remove => {
            if target == root {
                return Err(TreeError::RemoveRoot);
            }
            detach(arena, target, journal, removed);
        }
        StructOp::Move { to, at } => {
            let mut top = *to;
            let mut cycle = false;
            if arena.get(*to).is_none() {
                return Err(TreeError::StaleNode(*to));
            }
            for ancestor in to.ancestors(arena) {
                cycle |= ancestor == target;
                top = ancestor;
            }
            if top != root {
                return Err(not_in_tree(*to, top, removed));
            }
            if cycle {
                return Err(TreeError::Cycle {
                    node: target,
                    to: *to,
                });
            }
            journal.record(arena, target);
            journal.note_detach(arena, target);
            target.detach(arena);
            let key = arena[target].get().key();
            let prev = siblings::slot(arena, *to, &key, *at, E::key)?;
            journal.note_attach(arena, *to, prev);
            siblings::attach_at(arena, *to, target, prev);
        }
        StructOp::Split { at } => {
            let node = &arena[target];
            let Some(parent) = node.parent() else {
                return Err(TreeError::Unsplittable(target));
            };
            let element = node.get();
            let Some((first, second)) = element.split(at) else {
                return Err(TreeError::Unsplittable(target));
            };
            let key = element.key();
            if first.key() != key || second.key() != key {
                return Err(TreeError::PlacementCrossesKey);
            }
            journal.record(arena, target);
            *arena[target].get_mut() = first;
            let second = arena.new_node(second);
            journal.record_created(arena, second);
            journal.note_attach(arena, parent, Some(target));
            siblings::attach_at(arena, parent, second, Some(target));
        }
        StructOp::Merge => {
            let node = &arena[target];
            let Some(next) = node.next_sibling() else {
                return Err(TreeError::Unmergeable(target));
            };
            let element = node.get();
            let absorbed = &arena[next];
            let key = element.key();
            if absorbed.get().key() != key || absorbed.first_child().is_some() {
                return Err(TreeError::Unmergeable(target));
            }
            let Some(merged) = element.merge(absorbed.get()) else {
                return Err(TreeError::Unmergeable(target));
            };
            if merged.key() != key {
                return Err(TreeError::PlacementCrossesKey);
            }
            journal.record(arena, target);
            *arena[target].get_mut() = merged;
            detach(arena, next, journal, removed);
        }
    }
    Ok(())
}

/// Whether `node` is in the tree whose root is `root`: `Ok`, or the
/// error a command naming it gets. O(depth of `node`).
fn in_tree<E>(
    arena: &Arena<E>,
    root: NodeId,
    node: NodeId,
    removed: &FxHashSet<NodeId>,
) -> Result<(), TreeError> {
    if arena.get(node).is_none() {
        return Err(TreeError::StaleNode(node));
    }
    match node.ancestors(arena).last() {
        Some(top) if top == root => Ok(()),
        Some(top) => Err(not_in_tree(node, top, removed)),
        None => Err(TreeError::StaleNode(node)),
    }
}

/// The error for a command naming `node`, which is in the arena but in
/// a detached subtree whose top is `top`: the walk's own removal, or a
/// node that was out of the tree before the walk.
fn not_in_tree(node: NodeId, top: NodeId, removed: &FxHashSet<NodeId>) -> TreeError {
    if removed.contains(&top) {
        TreeError::TargetRemovedEarlier(node)
    } else {
        TreeError::StaleNode(node)
    }
}

/// Detach `node` and its subtree from the tree, recorded in `journal`,
/// and remember it as removed by this walk.
fn detach<E: TreeElement>(
    arena: &mut Arena<E>,
    node: NodeId,
    journal: &mut JournalBuilder<E>,
    removed: &mut FxHashSet<NodeId>,
) {
    journal.record(arena, node);
    journal.note_detach(arena, node);
    node.detach(arena);
    removed.insert(node);
}

/// Allocate `subtree` in `arena`, detached, each row in key order;
/// returns its root. Iterative, so a deep subtree cannot exhaust the
/// stack.
///
/// Cost: one arena slot per element, an end-of-row scan per child, and
/// a work list only when the root has children.
fn build_detached<E: TreeElement>(arena: &mut Arena<E>, subtree: &Subtree<E>) -> NodeId {
    let root = arena.new_node(subtree.root.clone());
    if subtree.children.is_empty() {
        return root;
    }
    let mut pending: Vec<(NodeId, &Subtree<E>)> = vec![(root, subtree)];
    while let Some((parent, node)) = pending.pop() {
        for child in &node.children {
            let id = arena.new_node(child.root.clone());
            let prev = siblings::last_slot(arena, parent, id, E::key);
            siblings::attach_at(arena, parent, id, prev);
            if !child.children.is_empty() {
                pending.push((id, child));
            }
        }
    }
    root
}
