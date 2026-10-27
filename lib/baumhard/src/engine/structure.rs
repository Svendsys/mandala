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
use crate::engine::tree::{climb, MutatorTree, Placement, Tree, TreeError};
use indextree::{Arena, NodeId};
use rustc_hash::FxHashSet;
use std::fmt;

/// A tree of elements that is not in any [`Tree`] yet — what a
/// [`StructOp::Insert`] puts in. `children` are in any order: each is
/// placed by its key, at the end of its run, so equal keys keep the
/// order given.
///
/// **Depth costs no call stack.** A subtree is as deep as whoever built
/// it made it — a whole branch of a map, which has no depth bound
/// (design D30, §20 rule 11) — so its [`Drop`], [`Clone`],
/// [`PartialEq`] and [`Debug`](fmt::Debug) are written by hand and
/// iterate with a heap stack, where derived ones would recurse once
/// per level. Not serializable: nothing writes a subtree (a
/// [`Mutator::Structure`] is never authored, #234), and serde's derive
/// would recurse the same way.
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

/// Drops level by level: the children of every subtree dropped are
/// moved onto a heap stack first, so each one is dropped childless.
/// Cost: O(elements); the stack holds at most the widest frontier.
impl<E> Drop for Subtree<E> {
    fn drop(&mut self) {
        if self.children.iter().all(|child| child.children.is_empty()) {
            // Leaves only: each drops with no children of its own.
            return;
        }
        let mut pending: Vec<Subtree<E>> = std::mem::take(&mut self.children);
        while let Some(mut subtree) = pending.pop() {
            pending.append(&mut subtree.children);
        }
    }
}

/// Clones element by element in pre-order, each subtree assembled once
/// its children are: the same allocations a derived clone makes, on a
/// heap stack as deep as the subtree. Cost: O(elements).
impl<E: Clone> Clone for Subtree<E> {
    fn clone(&self) -> Self {
        // The subtree being copied, the index of its next child, and
        // its element, cloned on the way down; finished subtrees wait
        // in `built` for their parent to collect them.
        let mut stack: Vec<(&Subtree<E>, usize, E)> = Vec::new();
        let mut built: Vec<Subtree<E>> = Vec::new();
        let mut current = (self, 0, self.root.clone());
        loop {
            if let Some(child) = current.0.children.get(current.1) {
                current.1 += 1;
                stack.push(std::mem::replace(&mut current, (child, 0, child.root.clone())));
                continue;
            }
            let children = built.split_off(built.len() - current.0.children.len());
            let finished = Subtree {
                root: current.2,
                children,
            };
            match stack.pop() {
                Some(parent) => {
                    built.push(finished);
                    current = parent;
                }
                None => return finished,
            }
        }
    }
}

/// Equal when the elements are equal pairwise in pre-order and every
/// subtree has as many children as its counterpart. Cost:
/// O(elements) at most; stops at the first difference.
impl<E: PartialEq> PartialEq for Subtree<E> {
    fn eq(&self, other: &Self) -> bool {
        let mut pending: Vec<(&Subtree<E>, &Subtree<E>)> = vec![(self, other)];
        while let Some((a, b)) = pending.pop() {
            if a.root != b.root || a.children.len() != b.children.len() {
                return false;
            }
            pending.extend(a.children.iter().zip(&b.children).rev());
        }
        true
    }
}

/// The derived form, written element by element in pre-order with a
/// heap stack instead of one call per level: `{:?}` gives
/// `Subtree { root: …, children: [Subtree { … }, …] }`, and `{:#?}` the
/// derived pretty form, a field per line and four spaces per level of
/// nesting — each element's own `{:#?}` lines indented with it, as the
/// derived form's padding indents them. Cost: O(elements); for `{:#?}`,
/// one `String` per element for its own lines.
impl<E: fmt::Debug> fmt::Debug for Subtree<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            return self.fmt_pretty(f);
        }
        let mut stack: Vec<(&Subtree<E>, usize)> = Vec::new();
        let mut current = (self, 0);
        f.write_str("Subtree { root: ")?;
        fmt::Debug::fmt(&self.root, f)?;
        f.write_str(", children: [")?;
        loop {
            if let Some(child) = current.0.children.get(current.1) {
                if current.1 > 0 {
                    f.write_str(", ")?;
                }
                current.1 += 1;
                f.write_str("Subtree { root: ")?;
                fmt::Debug::fmt(&child.root, f)?;
                f.write_str(", children: [")?;
                stack.push(std::mem::replace(&mut current, (child, 0)));
                continue;
            }
            f.write_str("] }")?;
            match stack.pop() {
                Some(parent) => current = parent,
                None => return Ok(()),
            }
        }
    }
}

impl<E: fmt::Debug> Subtree<E> {
    /// `{:#?}`: see the `Debug` impl. Each entry of the stack is a
    /// subtree with children, the index of its next child and its level
    /// of nesting, in units of four spaces.
    fn fmt_pretty(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        open_pretty(f, self, 0)?;
        if self.children.is_empty() {
            return Ok(());
        }
        let mut stack: Vec<(&Subtree<E>, usize, usize)> = Vec::new();
        let mut current = (self, 0, 0);
        loop {
            if let Some(child) = current.0.children.get(current.1) {
                current.1 += 1;
                let level = current.2 + 2;
                pad(f, level)?;
                open_pretty(f, child, level)?;
                if child.children.is_empty() {
                    f.write_str(",\n")?;
                } else {
                    stack.push(std::mem::replace(&mut current, (child, 0, level)));
                }
                continue;
            }
            pad(f, current.2 + 1)?;
            f.write_str("],\n")?;
            pad(f, current.2)?;
            f.write_str("}")?;
            match stack.pop() {
                Some(parent) => {
                    f.write_str(",\n")?;
                    current = parent;
                }
                None => return Ok(()),
            }
        }
    }
}

/// Four spaces per level, as the derived pretty form pads.
fn pad(f: &mut fmt::Formatter<'_>, level: usize) -> fmt::Result {
    for _ in 0..level {
        f.write_str("    ")?;
    }
    Ok(())
}

/// Write `subtree` at `level` in the pretty form up to its children:
/// its name, its root on its own lines, and either the empty list and
/// the closing brace, or the opening of a list its children follow.
fn open_pretty<E: fmt::Debug>(f: &mut fmt::Formatter<'_>, subtree: &Subtree<E>, level: usize) -> fmt::Result {
    f.write_str("Subtree {\n")?;
    pad(f, level + 1)?;
    f.write_str("root: ")?;
    let root = format!("{:#?}", subtree.root);
    let mut lines = root.split('\n');
    if let Some(first) = lines.next() {
        f.write_str(first)?;
    }
    for line in lines {
        f.write_str("\n")?;
        pad(f, level + 1)?;
        f.write_str(line)?;
    }
    f.write_str(",\n")?;
    pad(f, level + 1)?;
    if subtree.children.is_empty() {
        f.write_str("children: [],\n")?;
        pad(f, level)?;
        f.write_str("}")
    } else {
        f.write_str("children: [\n")
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
    /// Cost: one arena slot per subtree element, each put in place by a
    /// value insert that climbs nothing, plus a scan of the target's row
    /// for the placement.
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
    /// Cost: O(1) beside the journal's record of the target; when a
    /// later command of the same walk follows, one hash insert per
    /// element removed, once, so that command can tell what left the
    /// tree without climbing to the root.
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
    /// Cost: to a new parent, O(depth of `to`) to check for a cycle —
    /// the price of refusing one, paid once per node moved, never per
    /// node visited (ruling R16 on #282) — plus a scan of `to`'s row
    /// for the placement and the re-attach, which indextree checks the
    /// same way. Within its own row, no climb: the scan and a same-row
    /// re-attach.
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
/// Every target was reached by the walk, from anchors in the tree, so
/// it was in the tree when the commands began; a command finds it gone
/// only if an earlier one removed it, which the tree's O(1)
/// [`Tree::contains`] answers — no target is checked by climbing to
/// the root.
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

/// Run one command on `target`, which was in the tree when the walk
/// reached it. `removed` holds the tops of the subtrees this walk's
/// earlier commands detached.
fn apply_op<E: TreeElement>(
    tree: &mut Tree<E>,
    target: NodeId,
    op: &StructOp<E>,
    journal: &mut JournalBuilder<E>,
    removed: &mut FxHashSet<NodeId>,
) -> Result<(), TreeError> {
    if tree.arena.get(target).is_none() {
        return Err(TreeError::StaleNode(target));
    }
    if !tree.contains(target) {
        // In the tree when the walk reached it, so out of it only by an
        // earlier command of this walk.
        return Err(TreeError::TargetRemovedEarlier(target));
    }
    let root = tree.root;
    let Tree { arena, detached, .. } = tree;
    match op {
        StructOp::Insert { subtree, at } => {
            let prev = siblings::slot(arena, target, &subtree.root.key(), *at, E::key)?;
            journal.note_attach(arena, target, prev);
            let inserted = siblings::insert_value_at(arena, target, subtree.root.clone(), prev);
            journal.record_created(arena, inserted);
            build_below(arena, inserted, subtree);
        }
        StructOp::Remove => {
            if target == root {
                return Err(TreeError::RemoveRoot);
            }
            detach(arena, detached, target, journal, removed);
        }
        StructOp::Move { to, at } => {
            if arena.get(*to).is_none() {
                return Err(TreeError::StaleNode(*to));
            }
            let from = arena[target].parent();
            if from == Some(*to) {
                // A move within its own row: the parent stays, so
                // nothing can cycle and nothing needs climbing.
                journal.record(arena, target);
                journal.note_detach(arena, target);
                target.detach(arena);
                let key = arena[target].get().key();
                let prev = siblings::slot(arena, *to, &key, *at, E::key)?;
                journal.note_attach(arena, *to, prev);
                siblings::reattach_in_row(arena, *to, target, prev);
                return Ok(());
            }
            if detached.contains(to) {
                // Out of the tree: removed by this walk, or held by a
                // journal from before it. Which one is told by the top
                // of its detached subtree — a climb on this refusal
                // only.
                let top = climb(arena, *to).last().unwrap_or(*to);
                return Err(if removed.contains(&top) {
                    TreeError::TargetRemovedEarlier(*to)
                } else {
                    TreeError::StaleNode(*to)
                });
            }
            // A real re-parent: the one climb a command makes, to refuse
            // a cycle — once per node moved, the price of the move
            // (ruling R16 on #282), never per node visited.
            if climb(arena, *to).any(|ancestor| ancestor == target) {
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
            journal.note_attach(arena, parent, Some(target));
            let second = siblings::insert_value_at(arena, parent, second, Some(target));
            journal.record_created(arena, second);
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
            detach(arena, detached, next, journal, removed);
        }
    }
    Ok(())
}

/// Detach `node` and its subtree from the tree, recorded in `journal`;
/// mark every element of it out of the tree in `detached` (the tree's
/// set), and remember it as removed by this walk. O(subtree): the
/// elements a journal's touches name for this removal.
fn detach<E: TreeElement>(
    arena: &mut Arena<E>,
    detached: &mut FxHashSet<NodeId>,
    node: NodeId,
    journal: &mut JournalBuilder<E>,
    removed: &mut FxHashSet<NodeId>,
) {
    journal.record(arena, node);
    journal.note_detach(arena, node);
    node.detach(arena);
    detached.extend(node.descendants(arena));
    removed.insert(node);
}

/// Allocate the elements below `subtree`'s root under `top`, the node
/// already made for that root, each row in key order. Every node is
/// new, so each goes in through indextree's value inserts, which climb
/// nothing; and the work list is a heap stack, so a deep subtree costs
/// no call stack.
///
/// Cost: one arena slot per element, an end-of-row scan per child, and
/// a work list only when the root has children.
fn build_below<E: TreeElement>(arena: &mut Arena<E>, top: NodeId, subtree: &Subtree<E>) {
    if subtree.children.is_empty() {
        return;
    }
    let mut pending: Vec<(NodeId, &Subtree<E>)> = vec![(top, subtree)];
    while let Some((parent, node)) = pending.pop() {
        for child in &node.children {
            let id = siblings::append_value_sorted(arena, parent, child.root.clone(), E::key);
            if !child.children.is_empty() {
                pending.push((id, child));
            }
        }
    }
}
