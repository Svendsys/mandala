// SPDX-License-Identifier: MPL-2.0

//! The walker behind [`MutatorTree::apply_to`](crate::engine::tree::MutatorTree::apply_to) and
//! [`MutatorTree::apply_at`](crate::engine::tree::MutatorTree::apply_at): aligns mutators to
//! target-tree elements by [key](crate::engine::key::BranchKey) — kind and channel — and
//! dispatches each mutator's effect. Handles straight-line alignment,
//! [`Instruction::RepeatWhile`](crate::engine::mutator::Instruction::RepeatWhile) loop expansion (with the
//! [`Predicate`](crate::engine::predicate::Predicate) language), [`Instruction::MapChildren`](crate::engine::mutator::Instruction::MapChildren)'s positional
//! zip and [`Instruction::SpatialDescend`](crate::engine::mutator::Instruction::SpatialDescend)'s hit-test.
//!
//! # The merge
//!
//! A mutator child reaches a target child when their keys are equal.
//! Both trees keep every parent's children in ascending key order (the
//! sorted-children invariant, [`crate::engine::tree`]), so the walker
//! aligns a mutator row with a target row by walking the two sibling
//! lists **in step** through `first_child` / `next_sibling`: the lower
//! key advances, equal keys walk. Nothing is collected and nothing is
//! sorted.
//!
//! # Depth costs no call stack
//!
//! Neither tree's depth has a bound (design D30, §20 rule 11): a map
//! can be as deep as memory allows, and a mutator tree can mirror a
//! path to any element of it. So the walk does not recurse. Each step
//! of the descent — a (target, mutator) pair, a row being merged or
//! zipped, a `RepeatWhile` level, the terminator of a loop — is a
//! `Frame` on a heap stack in the walk, and the walk runs until the
//! stack is empty. A frame that reaches a child pushes itself back with
//! its cursors advanced, then the child's frame on top of it, so the
//! child's whole subtree is walked before the row moves on: the order
//! in which elements are tested, applied to and recorded is the
//! depth-first order a recursive walk has, step for step.
//!
//! # Inherited state
//!
//! Every frame names the [inherited state](crate::engine::element::TreeElement::Inherited)
//! of its target's **parent**. A mutation applied to the target
//! receives it; when the walk descends, the target's own state is
//! computed with [`TreeElement::inherit`](crate::engine::element::TreeElement::inherit)
//! *after* the target's own mutations, so a mutation sees its ancestors
//! as the same walk left them. The states live on a second heap stack
//! beside the frames: the frame that descends into a row pushes the
//! state its children receive and pops it when the row is done, so each
//! state is computed once per element descended through, read by
//! reference, and never cloned — nothing is stored on an element or in
//! a tree, and nothing is re-derived by climbing to the root.
//!
//! # Keys a walk changes
//!
//! A mutation may change its target's key. The walker compares the key
//! before and after every apply; when it changed, it records the
//! target in its `Walk` and carries on over the row **as it was**:
//! no row is re-sorted while the walk is still merging over it. When
//! the walk is over, `Walk::settle` moves exactly the recorded
//! children to their sorted places. A walk that changes no key records
//! nothing and allocates nothing for it.
//!
//! # Structural commands
//!
//! A [`Mutator::Structure`](crate::engine::mutator::Mutator::Structure)
//! the walk reaches changes nothing while the walk runs: the walk
//! collects it with its target, in the order it reaches them, and runs
//! the [commands](crate::engine::structure) once it has settled its
//! keys — its second phase. A walk whose mutator tree has a command
//! always records into a journal, its caller's or one of its own, so
//! that a failing command undoes the whole walk.
//!
//! # Journaled walks
//!
//! A journaled walk carries a
//! [`JournalBuilder`](crate::engine::journal::JournalBuilder) in its
//! `Walk`. The one function every apply goes through records the
//! target's before-image there immediately before its first apply, and
//! the settle notes every move it makes, so the
//! [journal](crate::engine::journal::Journal) can undo the walk exactly.
//! Mutations take no part in it.
//!
//! # Cache invalidation
//!
//! Every time the walker applies a mutator to a target it invalidates
//! the target tree's
//! [`TreeCache`](crate::engine::element::TreeCache) (one O(1) call),
//! so a caller that drives [`walk_tree_from`](crate::engine::walker::walk_tree_from()) directly sees no stale
//! derived state afterwards. Nothing inside a walk reads the cache.
//!
//! The entry points are [`walk_tree`](crate::engine::walker::walk_tree()) and [`walk_tree_from`](crate::engine::walker::walk_tree_from()),
//! beside [`MutatorTree::apply_to`](crate::engine::tree::MutatorTree::apply_to) and
//! [`MutatorTree::apply_at`](crate::engine::tree::MutatorTree::apply_at). Every one of them settles
//! the keys its walk changed and runs its structural commands before it
//! returns (`run_walk`), so a walk can only be started where the
//! sorted-children invariant is restored at its end; everything else in
//! this module is internal to the engine.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::journal::JournalBuilder;
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::siblings;
use crate::engine::structure::apply_ops;
use crate::engine::tree::{MutatorTree, Tree, TreeError};
use crate::util::geometry::aabb_contains;
use crate::util::ordered_vec2::OrderedVec2;
use indextree::{Arena, Node, NodeId};
use log::{debug, error, warn};
use std::cmp::Ordering;

/// What a walk keeps beside the two trees: what it
/// [records](WalkLog) — the children whose key it changed, the
/// structural commands it reached and its journal — and the two heap
/// stacks its descent runs on in place of the call stack: its
/// [frames](Frame), and the inherited states those frames hand down.
///
/// The stacks start empty and unallocated, grow to the depth the walk
/// reaches, and are reused by every anchor of the walk, so a descent
/// allocates nothing per visited element.
#[derive(Debug)]
pub(in crate::engine) struct Walk<'j, E: TreeElement> {
    rec: WalkLog<'j, E>,
    frames: Vec<Frame<E::Kind>>,
    states: Vec<E::Inherited>,
}

/// What a walk records beside the trees: the children whose key it
/// changed, recorded so they can be moved to their sorted places once
/// the walk is over ([`Walk::settle`]); the structural commands it
/// reached, collected for its second phase; and — for a journaled walk —
/// the [`JournalBuilder`] every apply is recorded in.
///
/// Each re-keyed record is `(parent, child)`, and each command
/// `(target, mutator node)`. Both `Vec`s stay empty — and unallocated —
/// for a walk that changes no key and reaches no command, which is the
/// common path. A child recorded twice (its key changed by two applies)
/// is moved once.
#[derive(Debug)]
struct WalkLog<'j, E: TreeElement> {
    rekeyed: Vec<(NodeId, NodeId)>,
    ops: Vec<(NodeId, NodeId)>,
    journal: Option<&'j mut JournalBuilder<E>>,
}

/// Where a frame finds the inherited state of its target's parent.
#[derive(Clone, Copy, Debug)]
enum StateRef {
    /// The state the walk was started with: the anchor's parent's.
    Seed,
    /// The state at this index of the walk's state stack, pushed by the
    /// frame below that descended into the row.
    Slot(usize),
}

/// One step of a walk's descent, on the walk's heap stack. Each variant
/// is the body of what a recursive walk would have as a function, with
/// its loop cursors made explicit; `own` is the index of the state the
/// frame pushed for its children, which it pops when it is done.
#[derive(Clone, Copy, Debug)]
enum Frame<K> {
    /// One (target, mutator) pair: apply the mutator's payload to the
    /// target when `apply_here`, then descend — by key for a `Single`,
    /// `Macro`, `Structure` or `Void`, through the instruction for an
    /// `Instruction`.
    Pair {
        target: NodeId,
        mutator: NodeId,
        state: StateRef,
        apply_here: bool,
    },
    /// A target row merged against a mutator row by key, each pair with
    /// equal keys visited in turn: as a [`Frame::Pair`], or — below a
    /// `RepeatWhile` instruction, the node `repeat` names — as the first
    /// level of its loop, a [`Frame::Repeat`].
    Merge {
        target: Option<NodeId>,
        mutator: Option<NodeId>,
        own: usize,
        repeat: Option<NodeId>,
    },
    /// A target row zipped against a mutator row by position
    /// ([`Instruction::MapChildren`]), each pair visited in turn as a
    /// [`Frame::Pair`] that applies whatever the keys.
    Zip {
        target: Option<NodeId>,
        mutator: Option<NodeId>,
        own: usize,
        paired: usize,
    },
    /// One level of a [`Instruction::RepeatWhile`] loop: test the
    /// predicate of the `instruction` node on the target; on a match,
    /// apply the repeated mutator and go on to the target's children of
    /// its kind; otherwise run the [terminator](Frame::Terminator).
    Repeat {
        target: NodeId,
        mutator: NodeId,
        instruction: NodeId,
        state: StateRef,
    },
    /// The children of a target a `RepeatWhile` matched, each of the
    /// repeated mutator's `kind` taken through a [`Frame::Repeat`] in
    /// turn; a child of another kind is passed.
    RepeatChildren {
        target: Option<NodeId>,
        mutator: NodeId,
        instruction: NodeId,
        kind: K,
        own: usize,
    },
    /// The continuation of a `RepeatWhile` loop whose predicate failed
    /// on `target`: the repeated mutator's own children whose key is
    /// the target's `key` — read when the loop ended — each walked from
    /// the target as a [`Frame::Pair`], in sibling order, up to the
    /// first key above it.
    Terminator {
        target: NodeId,
        after: Option<NodeId>,
        key: BranchKey<K>,
        state: StateRef,
    },
}

/// Run one walk to its end against `tree`: `body` walks — from the
/// root, or from each anchor — then the walk settles the keys it
/// changed and runs the [structural commands](crate::engine::structure)
/// it collected. Every walk entry point goes through here.
///
/// With `journal`, everything is recorded there, and an error is
/// returned with the builder as it stands: the journaled entry point
/// unwinds it. Without one, a mutator tree with no structural command
/// walks unrecorded, as it always has. One with structural commands
/// records into a builder of its own, because a failing command has to
/// undo the whole walk: on an error that builder is unwound — the tree
/// is as it was — and on success its journal is discarded, freeing
/// every subtree the walk removed, which nothing could put back.
///
/// Cost: the walk's, plus the settle and the commands; for an
/// unjournaled structural walk, what a journaled walk costs.
pub(in crate::engine) fn run_walk<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    journal: Option<&mut JournalBuilder<E>>,
    body: impl FnOnce(&mut Tree<E>, &mut Walk<'_, E>),
) -> Result<(), TreeError> {
    if journal.is_some() || !mutator_tree.is_structural() {
        let mut walk = Walk::new(journal);
        body(tree, &mut walk);
        return walk.finish(tree, mutator_tree);
    }
    let mut own = JournalBuilder::new();
    let mut walk = Walk::new(Some(&mut own));
    body(tree, &mut walk);
    let result = walk.finish(tree, mutator_tree);
    match result {
        Ok(()) => own.close(tree).discard(tree),
        Err(_) => own.unwind(tree),
    }
    result
}

impl<'j, E: TreeElement> Walk<'j, E> {
    /// A walk that records into `journal`, or into nothing. O(1); no
    /// allocation.
    pub(in crate::engine) fn new(journal: Option<&'j mut JournalBuilder<E>>) -> Self {
        Walk {
            rec: WalkLog {
                rekeyed: Vec::new(),
                ops: Vec::new(),
                journal,
            },
            frames: Vec::new(),
            states: Vec::new(),
        }
    }

    /// End the walk: [settle](Self::settle) the keys it changed, then
    /// run the structural commands it collected, in the order it
    /// reached them, recorded in its journal. A command's error is
    /// returned as is; undoing what came before it is the caller's.
    ///
    /// A walk can collect a command only from a mutator tree that has
    /// one, and such a walk always has a journal ([`run_walk`]). One
    /// without is an engine defect: its commands are dropped with an
    /// error logged, since nothing could undo them.
    fn finish<M: Mutation<E>>(
        mut self,
        tree: &mut Tree<E>,
        mutator_tree: &MutatorTree<E, M>,
    ) -> Result<(), TreeError> {
        let ops = std::mem::take(&mut self.rec.ops);
        let journal = self.settle(tree);
        if ops.is_empty() {
            return Ok(());
        }
        let Some(journal) = journal else {
            error!("engine: a walk without a journal reached structural commands; they are not applied");
            debug_assert!(false, "engine invariant: a structural walk has a journal");
            return Ok(());
        };
        apply_ops(tree, mutator_tree, &ops, journal)
    }

    /// Move every re-keyed child to its sorted place under its parent:
    /// after every sibling keyed at or below its new key. Every
    /// recorded child is detached first, which leaves each affected row
    /// sorted — the children that kept their keys were in order before
    /// the walk and nothing reordered them — and each is then placed
    /// into that sorted row in the order the walk recorded it. Children
    /// keep their ids, their parents and their subtrees; nothing is
    /// sorted. A journaled walk notes each detach and attach in its
    /// journal, which is how the journal can put the children back.
    ///
    /// A child that is the only one in its row is already in its sorted
    /// place whatever its key, and the detach and re-attach would put it
    /// back where it is, so it is left alone — which makes a walk that
    /// re-keys every element of a chain settle in O(1) per element.
    ///
    /// Hands back the walk's journal, for the commands that follow.
    ///
    /// Invalidates `tree`'s cache when anything was recorded. Cost:
    /// nothing when nothing was recorded; otherwise, per recorded child,
    /// an O(1) only-child check and, for a child with siblings, a
    /// detach, a scan from the end of its row and a same-row re-attach
    /// ([`siblings::reattach_in_row`]).
    fn settle(self, tree: &mut Tree<E>) -> Option<&'j mut JournalBuilder<E>> {
        let WalkLog {
            mut rekeyed,
            mut journal,
            ..
        } = self.rec;
        if rekeyed.is_empty() {
            return journal;
        }
        let arena = &mut tree.arena;
        // Before anything is detached: a child alone in its row has no
        // sibling recorded beside it, so leaving it is exact. Decided
        // first, because a detach below could leave a child alone that
        // was not.
        rekeyed.retain(|&(_, child)| {
            arena
                .get(child)
                .is_some_and(|n| n.previous_sibling().is_some() || n.next_sibling().is_some())
        });
        // `retain` visits every record once, in order: the first record
        // of a child detaches it, and a later one finds it detached and
        // drops out.
        rekeyed.retain(|&(parent, child)| {
            if arena.get(child).and_then(|n| n.parent()) == Some(parent) {
                if let Some(journal) = journal.as_deref_mut() {
                    journal.note_detach(arena, child);
                }
                child.detach(arena);
                true
            } else {
                false
            }
        });
        for &(parent, child) in &rekeyed {
            let prev = siblings::last_slot(arena, parent, child, E::key);
            if let Some(journal) = journal.as_deref_mut() {
                journal.note_attach(arena, parent, prev);
            }
            siblings::reattach_in_row(arena, parent, child, prev);
        }
        tree.cache.invalidate();
        journal
    }
}

/// Walk the entire `mutator_tree` against `tree`, starting from both
/// roots, with `seed` as the inherited state of the tree root's parent.
/// Convenience wrapper around [`walk_tree_from`]; cost is O(matching
/// pairs) — see that function for the full analysis.
pub fn walk_tree<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    seed: &E::Inherited,
) {
    walk_tree_from(tree, mutator_tree, tree.root, mutator_tree.root, seed)
}

/// Dispatch one mutator node against one target node and walk into
/// aligned children. The pair's own mutation applies only when the two
/// [keys](crate::engine::key::BranchKey) are equal; the walk below the
/// pair runs either way, aligning each child row by key. `target_id`
/// and `mutator_id` must belong to the arenas that back `tree` and
/// `mutator_tree` respectively; `parent_state` is the inherited state
/// of `target_id`'s parent.
///
/// Children whose key the walk changed are moved to their sorted places
/// when it returns (`Walk::settle`), and the
/// [structural commands](crate::engine::structure) it reached run then.
/// Not journaled: a command that fails undoes the whole walk, and the
/// error is logged, as [`MutatorTree::apply_to`] does. A `target_id`
/// not in the tree ([`Tree::contains`]) — a node of a subtree a journal
/// holds detached among them — is refused the same way, with nothing
/// applied, as [`Tree::update`] and the anchored walks refuse it.
///
/// Cost: one O(depth) check that `target_id` is in the tree — once per
/// call, never per element — then O(sum of matching (target, mutator)
/// pairs) — every pairwise match is one apply; pruned branches are
/// free. The descent runs on heap stacks as deep as the walk goes, not
/// on the call stack (see the module docs).
pub fn walk_tree_from<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
) {
    if !tree.contains(target_id) {
        error!("engine: walk_tree_from refused node {target_id}, which is not in the tree");
        return;
    }
    let keys_match = get_mutator(&mutator_tree.arena, mutator_id).get().key()
        == get_target(&tree.arena, target_id).get().key();
    let result = run_walk(tree, mutator_tree, None, |tree, walk| {
        walk_from(
            tree,
            mutator_tree,
            target_id,
            mutator_id,
            parent_state,
            keys_match,
            walk,
        )
    });
    if let Err(e) = result {
        error!("engine: a structural command failed, so the walk was undone: {e}");
    }
}

/// Walk `mutator_tree` from its root at one anchor of an anchored walk:
/// when the root's **kind** equals the anchor's, the root applies to
/// the anchor — the channel is not compared — and the walk continues
/// below; at an anchor of another kind nothing is applied, at the
/// anchor or below it. A stale `anchor` reaches nothing. Changed keys
/// are recorded in `walk`; settling them is the caller's, once every
/// anchor has been walked.
pub(in crate::engine) fn walk_anchor<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    anchor: NodeId,
    seed: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    let Some(target) = tree.arena.get(anchor) else {
        return;
    };
    let root = mutator_tree.root;
    if get_mutator(&mutator_tree.arena, root).get().key().kind != target.get().key().kind {
        debug!("Anchor {anchor} is of another kind than the mutator root, skipping it.");
        return;
    }
    walk_from(tree, mutator_tree, anchor, root, seed, true, walk);
}

/// The walk of one (target, mutator) pair and everything below it:
/// apply the mutator's own payload to the target when `apply_here`,
/// then descend — by key for a `Single` / `Macro` / `Void`, through the
/// instruction for an `Instruction`. `seed` is the inherited state of
/// `target_id`'s parent.
///
/// Runs the walk's [frames](Frame) until none is left; the walk's
/// stacks are empty when it starts and when it returns.
fn walk_from<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    seed: &E::Inherited,
    apply_here: bool,
    walk: &mut Walk<'_, E>,
) {
    let Walk { rec, frames, states } = walk;
    debug_assert!(
        frames.is_empty() && states.is_empty(),
        "engine invariant: a walk starts on empty stacks"
    );
    frames.clear();
    states.clear();
    let mut driver = Driver {
        mutator_tree,
        seed,
        rec,
        frames,
        states,
    };
    driver.frames.push(Frame::Pair {
        target: target_id,
        mutator: mutator_id,
        state: StateRef::Seed,
        apply_here,
    });
    while let Some(frame) = driver.frames.pop() {
        match frame {
            Frame::Pair {
                target,
                mutator,
                state,
                apply_here,
            } => driver.pair(tree, target, mutator, state, apply_here),
            Frame::Merge {
                target,
                mutator,
                own,
                repeat,
            } => driver.merge(tree, target, mutator, own, repeat),
            Frame::Zip {
                target,
                mutator,
                own,
                paired,
            } => driver.zip(tree, target, mutator, own, paired),
            Frame::Repeat {
                target,
                mutator,
                instruction,
                state,
            } => driver.repeat(tree, target, mutator, instruction, state),
            Frame::RepeatChildren {
                target,
                mutator,
                instruction,
                kind,
                own,
            } => driver.repeat_children(tree, target, mutator, instruction, kind, own),
            Frame::Terminator {
                target,
                after,
                key,
                state,
            } => driver.terminator(target, after, key, state),
        }
    }
    debug_assert!(
        driver.states.is_empty(),
        "engine invariant: every frame pops the state it pushed"
    );
}

/// The walk of one anchor in progress: the mutator tree, the anchor's
/// seed, and the walk's recorder and stacks, borrowed apart so a frame
/// can read a state while it records an apply. Each method is one
/// [`Frame`] variant's step: it is handed the frame popped off the
/// stack, and pushes back what remains of it, then the child it
/// reaches — so the child runs, subtree and all, before the frame goes
/// on.
struct Driver<'w, 'j, E: TreeElement, M: Mutation<E>> {
    mutator_tree: &'w MutatorTree<E, M>,
    seed: &'w E::Inherited,
    rec: &'w mut WalkLog<'j, E>,
    frames: &'w mut Vec<Frame<E::Kind>>,
    states: &'w mut Vec<E::Inherited>,
}

impl<E: TreeElement, M: Mutation<E>> Driver<'_, '_, E, M> {
    /// [`Frame::Pair`].
    fn pair(
        &mut self,
        tree: &mut Tree<E>,
        target: NodeId,
        mutator_id: NodeId,
        state: StateRef,
        apply_here: bool,
    ) {
        let mutator = get_mutator(&self.mutator_tree.arena, mutator_id).get();
        let parent_state = state_at(self.seed, self.states, state);
        match mutator {
            Mutator::Single { .. } | Mutator::Macro { .. } | Mutator::Structure { .. } => {
                debug!("Processing Delta Node...");
                if apply_here {
                    apply_mutator(tree, target, mutator_id, mutator, parent_state, self.rec);
                }
                self.align_child_walks(tree, target, mutator_id, state);
            }
            Mutator::Void { .. } => {
                debug!("Void mutator node, skipping");
                self.align_child_walks(tree, target, mutator_id, state);
            }
            Mutator::Instruction {
                instruction,
                mutation,
                ..
            } => {
                debug!("Processing Instruction node...");
                if apply_here && mutation.is_some() {
                    debug!("This instruction node has a Delta..");
                    apply_mutator(tree, target, mutator_id, mutator, parent_state, self.rec);
                }
                self.instruction(tree, target, mutator_id, instruction, state);
            }
        }
    }

    /// Start merging the children of the mutator and of the target by
    /// key ([`Frame::Merge`]): each pair with equal keys is walked as a
    /// pair. A mutator reaches every target with its key that the merge
    /// has not already passed: the first mutator on a key consumes that
    /// key's targets, so a later sibling mutator on the same key reaches
    /// none of them. The children receive `target`'s own state, computed
    /// here, after its own mutations.
    ///
    /// See also [`Instruction::MapChildren`] ([`Self::instruction`]) —
    /// the opt-out alternative that pairs children by sibling position
    /// (zip) instead of by key, for mutations that need per-index
    /// targeting.
    fn align_child_walks(&mut self, tree: &mut Tree<E>, target: NodeId, mutator: NodeId, state: StateRef) {
        debug!("Aligning children of target node {target} and mutator node {mutator}.");
        let Some(mutator_head) = get_mutator(&self.mutator_tree.arena, mutator).first_child() else {
            debug!("Mutator has no children - nothing to align.");
            return;
        };
        let Some(target_head) = get_target(&tree.arena, target).first_child() else {
            return;
        };
        let own = self.push_state(tree, target, state);
        self.frames.push(Frame::Merge {
            target: Some(target_head),
            mutator: Some(mutator_head),
            own,
            repeat: None,
        });
    }

    /// The descent through an instruction, whose own mutation (if any)
    /// has already been applied to `target`.
    ///
    /// - `RepeatWhile` merges the instruction's children against the
    ///   target's by key, each pair the first level of the loop
    ///   ([`Frame::Repeat`]).
    /// - `SpatialDescend` applies the instruction's mutation to the
    ///   descendant its point hits ([`spatial_descend`]); nothing below
    ///   it is walked.
    /// - `MapChildren` zips the instruction's children against the
    ///   target's by sibling position ([`Frame::Zip`]).
    fn instruction(
        &mut self,
        tree: &mut Tree<E>,
        target: NodeId,
        mutator: NodeId,
        instruction: &Instruction<E>,
        state: StateRef,
    ) {
        match instruction {
            Instruction::RepeatWhile(_) => {
                // Interactive path: a malformed RepeatWhile mutator (no
                // children to repeat) should degrade the walk, not abort
                // mutation application. The caller treats a no-op as
                // success.
                let Some(mutator_head) = get_mutator(&self.mutator_tree.arena, mutator).first_child() else {
                    warn!("engine::walker: RepeatWhile instruction node has no children, skipping branch");
                    return;
                };
                let Some(target_head) = get_target(&tree.arena, target).first_child() else {
                    debug!("The target has no children - completing walk down this branch.");
                    return;
                };
                let own = self.push_state(tree, target, state);
                self.frames.push(Frame::Merge {
                    target: Some(target_head),
                    mutator: Some(mutator_head),
                    own,
                    repeat: Some(mutator),
                });
            }
            Instruction::SpatialDescend(point) => {
                let parent_state = state_at(self.seed, self.states, state);
                spatial_descend(
                    tree,
                    self.mutator_tree,
                    target,
                    mutator,
                    point,
                    parent_state,
                    self.rec,
                );
            }
            Instruction::MapChildren => {
                let mutator_head = get_mutator(&self.mutator_tree.arena, mutator).first_child();
                let target_head = get_target(&tree.arena, target).first_child();
                let own = self.push_state(tree, target, state);
                self.frames.push(Frame::Zip {
                    target: target_head,
                    mutator: mutator_head,
                    own,
                    paired: 0,
                });
            }
        }
    }

    /// [`Frame::Merge`]: walk the two rows **in step** from their
    /// cursors — a target keyed below the current mutator is passed, a
    /// mutator keyed below the current target is passed — up to the
    /// next pair with equal keys, which is visited with the target
    /// passed. The mutator stays, so it reaches every consecutive target
    /// with its key (broadcast); the targets it reached are behind the
    /// merge, so a later sibling mutator with the same key reaches none
    /// of them.
    ///
    /// The next target is read before the visit runs. A visit changes
    /// only the visited target's subtree — and its key, whose re-sort
    /// waits for the end of the walk — so the row ahead of the merge is
    /// the row as it was.
    ///
    /// Cost: O(mutator row + target row) key reads over the frame's
    /// life, plus the visits; no allocation and no sort.
    fn merge(
        &mut self,
        tree: &mut Tree<E>,
        mut target: Option<NodeId>,
        mut mutator: Option<NodeId>,
        own: usize,
        repeat: Option<NodeId>,
    ) {
        while let (Some(m_id), Some(t_id)) = (mutator, target) {
            let m_node = get_mutator(&self.mutator_tree.arena, m_id);
            let t_node = get_target(&tree.arena, t_id);
            match t_node.get().key().cmp(&m_node.get().key()) {
                Ordering::Equal => {
                    self.frames.push(Frame::Merge {
                        target: t_node.next_sibling(),
                        mutator,
                        own,
                        repeat,
                    });
                    self.frames.push(match repeat {
                        None => Frame::Pair {
                            target: t_id,
                            mutator: m_id,
                            state: StateRef::Slot(own),
                            apply_here: true,
                        },
                        Some(instruction) => Frame::Repeat {
                            target: t_id,
                            mutator: m_id,
                            instruction,
                            state: StateRef::Slot(own),
                        },
                    });
                    return;
                }
                Ordering::Greater => {
                    debug!("Target key exceeds the mutator key, advancing to the next mutator.");
                    mutator = m_node.next_sibling();
                }
                Ordering::Less => {
                    target = t_node.next_sibling();
                }
            }
        }
        self.pop_state(own);
    }

    /// [`Frame::Zip`]: pair the next mutator child with the next target
    /// child by sibling position and walk the pair, the mutator's own
    /// effect **force-applied** to its target — bypassing the key match
    /// the merge enforces. That bypass is the point of
    /// [`Instruction::MapChildren`]: keys on the paired children are
    /// broadcast tags, and its job is to ignore them at the pairing
    /// site. Below the pair the walk is the standard one: by key, or
    /// through a nested instruction (a nested `MapChildren` zips one
    /// level deeper).
    ///
    /// Excess children on either side are dropped with a single
    /// `debug!` line when the zip ends. No allocation (§B7).
    fn zip(
        &mut self,
        tree: &mut Tree<E>,
        target: Option<NodeId>,
        mutator: Option<NodeId>,
        own: usize,
        paired: usize,
    ) {
        if let (Some(m_id), Some(t_id)) = (mutator, target) {
            self.frames.push(Frame::Zip {
                target: get_target(&tree.arena, t_id).next_sibling(),
                mutator: get_mutator(&self.mutator_tree.arena, m_id).next_sibling(),
                own,
                paired: paired + 1,
            });
            self.frames.push(Frame::Pair {
                target: t_id,
                mutator: m_id,
                state: StateRef::Slot(own),
                apply_here: true,
            });
            return;
        }
        // Count any leftover children on either side — useful when a
        // runtime-expanded mutator (via Repeat) disagrees with the actual
        // target fan-out and the author wants to see it in logs. Only one
        // of the two loops runs: the zip ended as soon as either side
        // ran dry.
        let mut excess_mutator: usize = 0;
        let mut cursor = mutator;
        while let Some(m) = cursor {
            excess_mutator += 1;
            cursor = self.mutator_tree.arena.get(m).and_then(|n| n.next_sibling());
        }
        let mut excess_target: usize = 0;
        let mut cursor = target;
        while let Some(t) = cursor {
            excess_target += 1;
            cursor = tree.arena.get(t).and_then(|n| n.next_sibling());
        }
        if excess_mutator > 0 || excess_target > 0 {
            debug!(
                "MapChildren zip paired {} children; {} excess mutators, {} excess targets ignored",
                paired, excess_mutator, excess_target
            );
        }
        self.pop_state(own);
    }

    /// [`Frame::Repeat`], the engine behind
    /// [`Instruction::RepeatWhile`]: as long as the predicate of the
    /// `instruction` node holds on the target, apply `mutator` to it and
    /// go on to its children ([`Frame::RepeatChildren`]); when it fails,
    /// resume the normal walk from the mutator's own children
    /// ([`Frame::Terminator`]).
    fn repeat(
        &mut self,
        tree: &mut Tree<E>,
        target: NodeId,
        mutator: NodeId,
        instruction: NodeId,
        state: StateRef,
    ) {
        let Mutator::Instruction {
            instruction: Instruction::RepeatWhile(condition),
            ..
        } = get_mutator(&self.mutator_tree.arena, instruction).get()
        else {
            error!("engine: a RepeatWhile frame names node {instruction}, which is no RepeatWhile");
            debug_assert!(
                false,
                "engine invariant: a Repeat frame's instruction is a RepeatWhile"
            );
            return;
        };
        if !condition.test(get_target(&tree.arena, target).get()) {
            // When a conditional loop terminates, both the mutator and
            // the target are where the predicate failed: the target has
            // not been mutated (yet), and the mutator is one step behind.
            debug!("The Terminator has received a mission.");
            self.frames.push(Frame::Terminator {
                target,
                after: get_mutator(&self.mutator_tree.arena, mutator).first_child(),
                key: get_target(&tree.arena, target).get().key(),
                state,
            });
            return;
        }
        debug!("Condition is met, applying mutator {mutator} to target {target}");
        let repeated = get_mutator(&self.mutator_tree.arena, mutator).get();
        let parent_state = state_at(self.seed, self.states, state);
        apply_mutator(tree, target, mutator, repeated, parent_state, self.rec);
        let Some(head) = get_target(&tree.arena, target).first_child() else {
            return;
        };
        let own = self.push_state(tree, target, state);
        // The descent is routed by kind, as every walk is: a child of
        // another kind than the repeated mutator is not tested, not
        // mutated and not descended into. The channel is not compared —
        // the descent broadcasts across channels on purpose.
        self.frames.push(Frame::RepeatChildren {
            target: Some(head),
            mutator,
            instruction,
            kind: repeated.key().kind,
            own,
        });
    }

    /// [`Frame::RepeatChildren`]: the next child of the repeated
    /// mutator's `kind` goes through the loop. The sibling link is read
    /// before the child's level runs, because that level may mutate the
    /// child it is handed and there must still be somewhere to go next.
    fn repeat_children(
        &mut self,
        tree: &mut Tree<E>,
        mut target: Option<NodeId>,
        mutator: NodeId,
        instruction: NodeId,
        kind: E::Kind,
        own: usize,
    ) {
        while let Some(id) = target {
            let child = get_target(&tree.arena, id);
            target = child.next_sibling();
            if child.get().kind() != kind {
                debug!("Child of another kind, not descending into it");
                continue;
            }
            debug!("Found child, descending into its sub-tree");
            self.frames.push(Frame::RepeatChildren {
                target,
                mutator,
                instruction,
                kind,
                own,
            });
            self.frames.push(Frame::Repeat {
                target: id,
                mutator,
                instruction,
                state: StateRef::Slot(own),
            });
            return;
        }
        self.pop_state(own);
    }

    /// [`Frame::Terminator`]: the after-mutations attached under the
    /// repeated mutator are in key order (the sorted-children
    /// invariant), so they are scanned in sibling order: every one whose
    /// key equals the target's — as it was when the loop ended — is
    /// walked from the target with the target's own parent state, and
    /// the scan stops at the first key above it. No allocation.
    fn terminator(
        &mut self,
        target: NodeId,
        mut after: Option<NodeId>,
        key: BranchKey<E::Kind>,
        state: StateRef,
    ) {
        while let Some(after_id) = after {
            let node = get_mutator(&self.mutator_tree.arena, after_id);
            after = node.next_sibling();
            match node.get().key().cmp(&key) {
                Ordering::Equal => {
                    debug!("Next mutator matches the target, starting walk..");
                    self.frames.push(Frame::Terminator {
                        target,
                        after,
                        key,
                        state,
                    });
                    self.frames.push(Frame::Pair {
                        target,
                        mutator: after_id,
                        state,
                        apply_here: true,
                    });
                    return;
                }
                Ordering::Greater => {
                    debug!("Next mutator key is above the target's, ending branch..");
                    return;
                }
                Ordering::Less => {}
            }
        }
        debug!("No more mutators, ending branch..");
    }

    /// Push the state `target` hands its children — its own
    /// [`TreeElement::inherit`] of its parent's, read after its own
    /// mutations — and return its slot. O(1) plus the element type's
    /// `inherit`; the stack grows only past the depth it has reached.
    fn push_state(&mut self, tree: &Tree<E>, target: NodeId, state: StateRef) -> usize {
        let own = get_target(&tree.arena, target)
            .get()
            .inherit(state_at(self.seed, self.states, state));
        self.states.push(own);
        self.states.len() - 1
    }

    /// Pop the state the finishing frame pushed at `own`, the top of the
    /// state stack: every frame above it has finished first.
    fn pop_state(&mut self, own: usize) {
        debug_assert_eq!(
            self.states.len(),
            own + 1,
            "engine invariant: a frame pops the state it pushed, last"
        );
        self.states.truncate(own);
    }
}

/// The state a [`StateRef`] names. O(1).
#[inline]
fn state_at<'s, I>(seed: &'s I, states: &'s [I], state: StateRef) -> &'s I {
    match state {
        StateRef::Seed => seed,
        StateRef::Slot(i) => &states[i],
    }
}

/// Apply `mutator`'s payload to `target_id` through [`apply_recorded`].
/// A mutator with no payload ([`Mutator::has_payload`]) — a `Void`, an
/// empty `Macro`, an `Instruction` without a mutation — applies
/// nothing, so it does not reach `apply_recorded`: the element it is
/// paired with is visited, not applied to, and is not journaled. A
/// [`Mutator::Structure`] — the node `mutator_id` — is not applied
/// either: the walk collects it with its target for its second phase.
#[inline]
fn apply_mutator<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    target_id: NodeId,
    mutator_id: NodeId,
    mutator: &Mutator<E, M>,
    parent_state: &E::Inherited,
    rec: &mut WalkLog<'_, E>,
) {
    if let Mutator::Structure { .. } = mutator {
        rec.ops.push((target_id, mutator_id));
        return;
    }
    if !mutator.has_payload() {
        return;
    }
    apply_recorded(tree, target_id, rec, |target| {
        mutator.apply_to(target, parent_state)
    });
}

/// Run `f` on the element at `target_id` — every apply in the walker
/// goes through here. A journaled walk first records the element in
/// its journal (a no-op after the element's first apply in that
/// journal); then `f` runs, the tree's cache is invalidated and, if `f`
/// changed the element's key, the element is recorded in `walk` to be
/// moved when the walk settles.
///
/// Cost: two O(1) key reads beside `f`; journaled, one hash lookup, and
/// on the element's first apply in the journal a clone of it.
#[inline]
fn apply_recorded<E: TreeElement>(
    tree: &mut Tree<E>,
    target_id: NodeId,
    rec: &mut WalkLog<'_, E>,
    f: impl FnOnce(&mut E),
) {
    if let Some(journal) = rec.journal.as_deref_mut() {
        journal.record(&tree.arena, target_id);
    }
    let node = get_target_mut(&mut tree.arena, target_id);
    let before = node.get().key();
    f(node.get_mut());
    if node.get().key() != before {
        if let Some(parent) = node.parent() {
            rec.rekeyed.push((parent, target_id));
        }
    }
    tree.cache.invalidate();
}

/// Look up a mutator-tree node by id.
///
/// **Precondition:** `id` must come from `mutator_tree.arena` (every
/// caller in this file walks the same arena it was handed). Violating
/// the precondition means the mutator tree is corrupted, which is a
/// bug at the call site, not user-recoverable input.
///
/// The `expect` is *not* an interactive-path violation under
/// CODE_CONVENTIONS.md §9: it asserts a tight internal invariant that,
/// if broken, means the walker is operating on inconsistent state and
/// continuing would silently corrupt the user's data. We prefer a
/// clean panic over a corrupt save.
#[inline]
fn get_mutator<E: TreeElement, M: Mutation<E>>(
    arena: &Arena<Mutator<E, M>>,
    id: NodeId,
) -> &Node<Mutator<E, M>> {
    arena
        .get(id)
        .expect("walker invariant: mutator NodeId must belong to mutator_tree.arena")
}

/// Look up a target-tree node by id, see [`get_mutator`] for the
/// invariant rationale. Same precondition: `id` must originate from
/// the target tree's arena.
#[inline]
fn get_target<E: TreeElement>(arena: &Arena<E>, id: NodeId) -> &Node<E> {
    arena
        .get(id)
        .expect("walker invariant: target NodeId must belong to tree.arena")
}

/// [`get_target`], mutably.
#[inline]
fn get_target_mut<E: TreeElement>(arena: &mut Arena<E>, id: NodeId) -> &mut Node<E> {
    arena
        .get_mut(id)
        .expect("walker invariant: target NodeId must belong to tree.arena")
}

// ── SpatialDescend ────────────────────────────────────────────────

/// Find the smallest descendant of `target_id` whose own box
/// ([`TreeElement::spatial_bounds`]) contains `point`, then apply the
/// instruction's attached mutation to it.
///
/// # Algorithm
///
/// 1. Visit every descendant of `target_id` (not `target_id` itself)
///    in pre-order, through `indextree`'s allocation-free
///    `descendants` traversal.
/// 2. Among the boxes that contain the point, keep the smallest by
///    area; the first one found keeps a tie (innermost-first, then
///    document order).
/// 3. Apply the instruction's mutation to that node, regardless of
///    key — the spatial match overrides key alignment — with the hit's
///    parent's inherited state, folded down the path from `target_id`.
///
/// If no box contains the point, the instruction is a no-op.
///
/// Reads only the elements' own boxes, never the tree's cache, so a
/// mutation applied earlier in the same walk is always seen.
///
/// Cost: O(descendants of `target_id`); no allocation on a miss, one
/// `Vec` of the hit's depth below `target_id` on a hit.
fn spatial_descend<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    point: &OrderedVec2,
    parent_state: &E::Inherited,
    rec: &mut WalkLog<'_, E>,
) {
    let point = point.to_vec2();
    let mut best: Option<(NodeId, f32)> = None;
    for id in target_id.descendants(&tree.arena).skip(1) {
        let Some(node) = tree.arena.get(id) else {
            continue;
        };
        let Some((top_left, size)) = node.get().spatial_bounds() else {
            continue;
        };
        if !aabb_contains(point, top_left, top_left + size) {
            continue;
        }
        let area = size.x * size.y;
        match best {
            Some((_, best_area)) if best_area <= area => {}
            _ => best = Some((id, area)),
        }
    }

    let Some((hit_id, _)) = best else {
        debug!("SpatialDescend: no node contains the point, no-op.");
        return;
    };
    debug!("SpatialDescend: hit node {:?}, applying mutation.", hit_id);

    let Mutator::Instruction {
        mutation: Some(mutation),
        ..
    } = get_mutator(&mutator_tree.arena, mutator_id).get()
    else {
        return;
    };
    let state = state_above(tree, target_id, hit_id, parent_state);
    apply_recorded(tree, hit_id, rec, |hit| mutation.apply_to(hit, &state));
}

/// The inherited state of `descendant`'s parent, where `descendant` is
/// a strict descendant of `ancestor` and `parent_state` is the state of
/// `ancestor`'s parent: [`TreeElement::inherit`] folded down the path
/// `ancestor ..= parent(descendant)`.
///
/// The path is collected bottom-up into one `Vec` and folded top-down.
/// Iterative on purpose — a target tree's depth is whatever its source
/// made it, and recursing over it is how a deep chain exhausts the
/// stack.
fn state_above<E: TreeElement>(
    tree: &Tree<E>,
    ancestor: NodeId,
    descendant: NodeId,
    parent_state: &E::Inherited,
) -> E::Inherited {
    let mut path: Vec<NodeId> = Vec::new();
    let mut cursor = tree.arena.get(descendant).and_then(|n| n.parent());
    while let Some(id) = cursor {
        path.push(id);
        if id == ancestor {
            break;
        }
        cursor = tree.arena.get(id).and_then(|n| n.parent());
    }
    let mut state = parent_state.clone();
    for id in path.iter().rev() {
        if let Some(node) = tree.arena.get(*id) {
            state = node.get().inherit(&state);
        }
    }
    state
}
