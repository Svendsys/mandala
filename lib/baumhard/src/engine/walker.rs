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
//! # Inherited state
//!
//! Every function here carries `parent_state: &E::Inherited` — the
//! [inherited state](crate::engine::element::TreeElement::Inherited) of the current target's
//! **parent**. A mutation applied to the target receives it; when the
//! walk descends, the target's own state is computed with
//! [`TreeElement::inherit`](crate::engine::element::TreeElement::inherit) *after* the target's own mutations, so a
//! mutation sees its ancestors as the same walk left them. The state
//! is a local of the recursion — nothing is stored on an element or in
//! a tree, and no visited node costs an allocation for it.
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
//! the keys its walk changed before it returns, so a walk can only be
//! started where the sorted-children invariant is restored at its end;
//! everything else in this module is internal to the engine.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::journal::JournalBuilder;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::siblings;
use crate::engine::tree::{MutatorTree, Tree};
use crate::util::geometry::aabb_contains;
use crate::util::ordered_vec2::OrderedVec2;
use indextree::{Arena, Node, NodeId};
use log::{debug, warn};
use std::cmp::Ordering;

/// What a walk keeps beside the two trees: the children whose key it
/// changed, recorded so they can be moved to their sorted places once
/// the walk is over ([`Walk::settle`]), and — for a journaled walk —
/// the [`JournalBuilder`] every apply is recorded in.
///
/// Each re-keyed record is `(parent, child)`. The `Vec` stays empty —
/// and unallocated — for a walk that changes no key, which is the
/// common path. A child recorded twice (its key changed by two applies)
/// is moved once.
#[derive(Debug)]
pub(in crate::engine) struct Walk<'j, E: TreeElement> {
    rekeyed: Vec<(NodeId, NodeId)>,
    journal: Option<&'j mut JournalBuilder<E>>,
}

impl<'j, E: TreeElement> Walk<'j, E> {
    /// A walk that records into `journal`, or into nothing. O(1); no
    /// allocation.
    pub(in crate::engine) fn new(journal: Option<&'j mut JournalBuilder<E>>) -> Self {
        Walk {
            rekeyed: Vec::new(),
            journal,
        }
    }

    /// Move every re-keyed child to its sorted place under its parent:
    /// after every sibling keyed at or below its new key. Every
    /// recorded child is detached first, which leaves each affected row
    /// sorted — the children that kept their keys were in order before
    /// the walk and nothing reordered them — and each is then placed
    /// into that sorted row in the order the walk recorded it. Children
    /// keep their ids and subtrees; nothing is sorted. A journaled walk
    /// notes each detach and attach in its journal, which is how the
    /// journal can put the children back.
    ///
    /// Invalidates `tree`'s cache when it moved anything. Cost: nothing
    /// when nothing was recorded; otherwise, per recorded child, a
    /// detach, a scan from the end of its row and an O(depth)
    /// re-attach.
    pub(in crate::engine) fn settle(self, tree: &mut Tree<E>) {
        let Walk {
            mut rekeyed,
            mut journal,
        } = self;
        if rekeyed.is_empty() {
            return;
        }
        let arena = &mut tree.arena;
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
            siblings::attach_at(arena, parent, child, prev);
        }
        tree.cache.invalidate();
    }
}

/// The continuation [`repeat_while`] calls when its predicate fails
/// on a target: `(tree, mutator_tree, target_id, mutator_id,
/// parent_state, walk)`, with `parent_state` the inherited state of
/// `target_id`'s parent and `walk` what the walk keeps beside the trees
/// ([`Walk`]). [`default_terminator`] is the one the walker uses.
pub(in crate::engine) type Terminator<E, M> =
    fn(&mut Tree<E>, &MutatorTree<E, M>, NodeId, NodeId, &<E as TreeElement>::Inherited, &mut Walk<'_, E>);

/// Continuation called after a conditional loop
/// ([`Instruction::RepeatWhile`]) exits — it resumes the normal walk
/// from the mutator's children whose key matches the current target.
///
/// The after-mutations attached under `mutator_id` are in key order
/// (the sorted-children invariant), so they are scanned in sibling
/// order: every one whose key equals the target's is dispatched with
/// the target's own `parent_state`, and the scan stops at the first
/// key above the target's. No allocation.
pub(in crate::engine) fn default_terminator<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    // When a conditional loop terminates, both the mutator and the
    // target are where the predicate failed: the target has not been
    // mutated (yet), and the mutator is one step behind.
    debug!("The Terminator has received a mission.");
    let target_key = get_target(&tree.arena, target_id).get().key();
    let mut after = get_mutator(&mutator_tree.arena, mutator_id).first_child();
    while let Some(after_id) = after {
        let node = get_mutator(&mutator_tree.arena, after_id);
        after = node.next_sibling();
        match node.get().key().cmp(&target_key) {
            Ordering::Equal => {
                debug!("Next mutator matches the target, starting walk..");
                walk_from(tree, mutator_tree, target_id, after_id, parent_state, true, walk);
            }
            Ordering::Greater => {
                debug!("Next mutator key is above the target's, ending branch..");
                break;
            }
            Ordering::Less => {}
        }
    }
    debug!("No more mutators, ending branch..");
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

/// Dispatch one mutator node against one target node and recurse into
/// aligned children. The pair's own mutation applies only when the two
/// [keys](crate::engine::key::BranchKey) are equal; the walk below the
/// pair runs either way, aligning each child row by key. `target_id`
/// and `mutator_id` must belong to the arenas that back `tree` and
/// `mutator_tree` respectively; `parent_state` is the inherited state
/// of `target_id`'s parent.
///
/// Children whose key the walk changed are moved to their sorted places
/// when it returns (`Walk::settle`). Not journaled.
///
/// Cost: O(sum of matching (target, mutator) pairs) — every pairwise
/// match is one apply; pruned branches are free.
pub fn walk_tree_from<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
) {
    let keys_match = get_mutator(&mutator_tree.arena, mutator_id).get().key()
        == get_target(&tree.arena, target_id).get().key();
    let mut walk = Walk::new(None);
    walk_from(
        tree,
        mutator_tree,
        target_id,
        mutator_id,
        parent_state,
        keys_match,
        &mut walk,
    );
    walk.settle(tree);
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

/// The walk of one (target, mutator) pair: apply the mutator's own
/// payload to the target when `apply_here`, then descend — by key for
/// a `Single` / `Macro` / `Void`, through the instruction for an
/// `Instruction`.
fn walk_from<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
    apply_here: bool,
    walk: &mut Walk<'_, E>,
) {
    let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();

    match mutator {
        Mutator::Single { .. } | Mutator::Macro { .. } => {
            debug!("Processing Delta Node...");
            if apply_here {
                apply_mutator(tree, target_id, mutator, parent_state, walk);
            }
        }
        Mutator::Void { .. } => {
            debug!("Void mutator node, skipping")
        }
        Mutator::Instruction {
            instruction,
            mutation,
            ..
        } => {
            debug!("Processing Instruction node...");
            if apply_here && mutation.is_some() {
                debug!("This instruction node has a Delta..");
                apply_mutator(tree, target_id, mutator, parent_state, walk);
            }
            process_instruction_node(
                tree,
                mutator_tree,
                target_id,
                mutator_id,
                instruction,
                parent_state,
                walk,
            );
            return;
        }
    }
    align_child_walks(tree, mutator_tree, target_id, mutator_id, parent_state, walk);
}

/// Apply `mutator`'s payload to `target_id` through [`apply_recorded`].
/// A mutator with no payload ([`Mutator::has_payload`]) — a `Void`, an
/// empty `Macro`, an `Instruction` without a mutation — applies
/// nothing, so it does not reach `apply_recorded`: the element it is
/// paired with is visited, not applied to, and is not journaled.
#[inline]
fn apply_mutator<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    target_id: NodeId,
    mutator: &Mutator<E, M>,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    if !mutator.has_payload() {
        return;
    }
    apply_recorded(tree, target_id, walk, |target| {
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
    walk: &mut Walk<'_, E>,
    f: impl FnOnce(&mut E),
) {
    if let Some(journal) = walk.journal.as_deref_mut() {
        journal.record(&tree.arena, target_id);
    }
    let node = get_target_mut(&mut tree.arena, target_id);
    let before = node.get().key();
    f(node.get_mut());
    if node.get().key() != before {
        if let Some(parent) = node.parent() {
            walk.rekeyed.push((parent, target_id));
        }
    }
    tree.cache.invalidate();
}

/// The inherited state `target_id` hands its children: its own
/// [`TreeElement::inherit`] of `parent_state`, read after its own
/// mutations. O(1) plus the element type's `inherit`.
#[inline]
fn own_state<E: TreeElement>(tree: &Tree<E>, target_id: NodeId, parent_state: &E::Inherited) -> E::Inherited {
    get_target(&tree.arena, target_id).get().inherit(parent_state)
}

#[inline]
fn process_instruction_node<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    instruction: &Instruction<E>,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    match instruction {
        Instruction::RepeatWhile(condition) => {
            let mutator = get_mutator(&mutator_tree.arena, mutator_id);
            let target = get_target(&tree.arena, target_id);
            // Interactive path: a malformed RepeatWhile mutator (no
            // children to repeat) should degrade the walk, not abort
            // mutation application. The caller treats a no-op as
            // success.
            if mutator.first_child().is_none() {
                warn!("engine::walker: RepeatWhile instruction node has no children, skipping branch");
                return;
            }
            if target.first_child().is_none() {
                debug!("The target has no children - completing walk down this branch.");
                return;
            }
            let state = own_state(tree, target_id, parent_state);
            merge_rows(tree, mutator_tree, target_id, mutator_id, |tree, t_id, m_id| {
                repeat_while(
                    tree,
                    mutator_tree,
                    t_id,
                    m_id,
                    condition,
                    default_terminator::<E, M>,
                    &state,
                    walk,
                );
            });
        }
        Instruction::SpatialDescend(point) => {
            spatial_descend(
                tree,
                mutator_tree,
                target_id,
                mutator_id,
                point,
                parent_state,
                walk,
            );
        }
        Instruction::MapChildren => {
            zip_map_children(tree, mutator_tree, target_id, mutator_id, parent_state, walk);
        }
    };
}

/// Merge the children of `target_parent_id` against the children of
/// `mutator_parent_id` by key, calling `visit(tree, target, mutator)`
/// for every pair with equal keys.
///
/// Both rows are in ascending key order (the sorted-children
/// invariant), so they are walked **in step** through `first_child` /
/// `next_sibling`: a target keyed below the current mutator is passed,
/// a mutator keyed below the current target is passed, and on equal
/// keys the pair is visited and the target passed. The mutator stays,
/// so it reaches every consecutive target with its key (broadcast);
/// the targets it reached are behind the merge, so a later sibling
/// mutator with the same key reaches none of them.
///
/// The next target is read before `visit` runs. A visit changes only
/// the visited target's subtree — and its key, whose re-sort waits for
/// the end of the walk — so the row ahead of the merge is the row as it
/// was.
///
/// Cost: O(mutator row + target row) key reads plus the visits; no
/// allocation and no sort.
#[inline]
fn merge_rows<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_parent_id: NodeId,
    mutator_parent_id: NodeId,
    mut visit: impl FnMut(&mut Tree<E>, NodeId, NodeId),
) {
    let mut mutator_cursor = get_mutator(&mutator_tree.arena, mutator_parent_id).first_child();
    let mut target_cursor = get_target(&tree.arena, target_parent_id).first_child();
    while let (Some(m_id), Some(t_id)) = (mutator_cursor, target_cursor) {
        let mutator = get_mutator(&mutator_tree.arena, m_id);
        let target = get_target(&tree.arena, t_id);
        match target.get().key().cmp(&mutator.get().key()) {
            Ordering::Equal => {
                target_cursor = target.next_sibling();
                visit(tree, t_id, m_id);
            }
            Ordering::Greater => {
                debug!("Target key exceeds the mutator key, advancing to the next mutator.");
                mutator_cursor = mutator.next_sibling();
            }
            Ordering::Less => {
                target_cursor = target.next_sibling();
            }
        }
    }
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

/// Take the children of the mutator and of the target, and start a
/// walk for each pair with equal keys ([`merge_rows`]). A mutator
/// reaches every target with its key that the merge has not already
/// passed: the first mutator on a key consumes that key's targets, so a
/// later sibling mutator on the same key reaches none of them.
/// `parent_state` is the inherited state of `target_id`'s parent; the
/// children receive `target_id`'s own state.
///
/// See also [`zip_map_children`] — the opt-out alternative that pairs
/// children by sibling position (zip) instead of by key, for mutations
/// that need per-index targeting.
///
/// Cost: O(mutator row + target row) key reads plus the per-pair
/// recursion; no allocation and no sort.
#[inline]
fn align_child_walks<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    debug!(
        "Aligning children of target node {} and mutator node {}.",
        target_id, mutator_id
    );
    if get_mutator(&mutator_tree.arena, mutator_id)
        .first_child()
        .is_none()
    {
        debug!("Mutator has no children - nothing to align.");
        return;
    }
    if get_target(&tree.arena, target_id).first_child().is_none() {
        return;
    }
    let state = own_state(tree, target_id, parent_state);
    merge_rows(tree, mutator_tree, target_id, mutator_id, |tree, t_id, m_id| {
        walk_from(tree, mutator_tree, t_id, m_id, &state, true, walk);
    });
}

/// Zip the mutator's direct children against the target's direct
/// children by sibling position — the alternative to
/// [`align_child_walks`] for consumers that need per-index targeting
/// independent of keys (e.g. size-aware layouts where every target
/// child sits on the same broadcast channel).
///
/// For each pair up to `min(mutator_children_len,
/// target_children_len)`, the mutator's own effect is
/// **force-applied** to its paired target — bypassing the key-match
/// check the merge normally enforces. That bypass is the whole point:
/// keys on the paired children are broadcast tags, and MapChildren's
/// job is to ignore them at the pairing site.
///
/// After the force-apply, the mutator's subtree descends against the
/// target's subtree through the standard path:
/// - Single/Macro/Void → [`align_child_walks`] (key-aware).
/// - Instruction → [`process_instruction_node`], which re-dispatches
///   into whichever instruction body the child carries (including a
///   nested MapChildren, which will zip one level deeper).
///
/// Excess children on either side are silently dropped with a single
/// `debug!` line at termination. No allocation inside the loop (§B7).
/// Graceful no-op on empty children on either side.
///
/// The *outer* instruction's own attached mutation is already applied
/// to the current target by [`walk_from`] before this function is
/// called (same precedent as `RepeatWhile`) — this function handles
/// only the descent into paired children, which receive `target_id`'s
/// own inherited state.
#[inline]
fn zip_map_children<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    let mut option_mutator_child = get_mutator(&mutator_tree.arena, mutator_id).first_child();
    let mut option_target_child = get_target(&tree.arena, target_id).first_child();
    let state = own_state(tree, target_id, parent_state);

    let mut paired: usize = 0;
    while let (Some(mutator_child_id), Some(target_child_id)) = (option_mutator_child, option_target_child) {
        // Look up the next-siblings *before* the force-apply that
        // takes `&mut tree.arena`, so the read-only borrows end
        // cleanly.
        let next_mutator = get_mutator(&mutator_tree.arena, mutator_child_id).next_sibling();
        let next_target = get_target(&tree.arena, target_child_id).next_sibling();

        // Force-apply the mutator to its paired target. The mutator is
        // borrowed from `mutator_tree`, which the dispatch below only
        // reads, so a nested instruction is handed on by reference.
        let mutator = get_mutator(&mutator_tree.arena, mutator_child_id).get();
        apply_mutator(tree, target_child_id, mutator, &state, walk);
        match mutator {
            Mutator::Instruction { instruction, .. } => {
                // Nested instruction: dispatch at the paired target.
                // Matches `walk_from`'s post-apply path for
                // Instruction (`process_instruction_node` + early
                // return — no align_child_walks).
                process_instruction_node(
                    tree,
                    mutator_tree,
                    target_child_id,
                    mutator_child_id,
                    instruction,
                    &state,
                    walk,
                );
            }
            Mutator::Single { .. } | Mutator::Macro { .. } | Mutator::Void { .. } => {
                // Descend via key-based align at the next level down.
                // A user who wants the deeper level to also zip nests
                // MapChildren inside.
                align_child_walks(
                    tree,
                    mutator_tree,
                    target_child_id,
                    mutator_child_id,
                    &state,
                    walk,
                );
            }
        }

        paired += 1;
        option_mutator_child = next_mutator;
        option_target_child = next_target;
    }

    // Count any leftover children on either side — useful when a
    // runtime-expanded mutator (via Repeat) disagrees with the actual
    // target fan-out and the author wants to see it in logs. Only one
    // of the two loops runs because we broke out as soon as either
    // side ran dry.
    let mut excess_mutator: usize = 0;
    while let Some(m) = option_mutator_child {
        excess_mutator += 1;
        option_mutator_child = mutator_tree.arena.get(m).and_then(|n| n.next_sibling());
    }
    let mut excess_target: usize = 0;
    while let Some(t) = option_target_child {
        excess_target += 1;
        option_target_child = tree.arena.get(t).and_then(|n| n.next_sibling());
    }
    if excess_mutator > 0 || excess_target > 0 {
        debug!(
            "MapChildren zip paired {} children; {} excess mutators, {} excess targets ignored",
            paired, excess_mutator, excess_target
        );
    }
}

/// As long as `condition` holds true, keep applying `mutator_id` (and
/// its descendants) to `target_id` (and its descendants). When the
/// condition fails, call `terminator` to resume the normal walk.
/// `parent_state` is the inherited state of `target_id`'s parent, and
/// `walk` the walk's record of changed keys.
///
/// This is the engine behind [`Instruction::RepeatWhile`]. Below
/// `target_id` it visits only the children of the repeated mutator's
/// **kind**, whatever their channel; a child of another kind is not
/// tested, not mutated and not descended into. The `terminator` —
/// [`default_terminator`] in every walk — receives the same tree pair,
/// the current target/mutator ids, `parent_state` and `walk`.
/// Internal to the engine: `walk` must be settled when the walk is
/// over, which only the walk's entry points do.
#[allow(clippy::too_many_arguments)]
pub(in crate::engine) fn repeat_while<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate<E>,
    terminator: Terminator<E, M>,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    let condition_matches = condition.test(get_target(&tree.arena, target_id).get());
    if condition_matches {
        debug!(
            "Condition is met, applying mutator {} to target {}",
            mutator_id, target_id
        );
        let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();
        apply_mutator(tree, target_id, mutator, parent_state, walk);
        apply_repeat_while_to_children(
            tree,
            mutator_tree,
            target_id,
            mutator_id,
            condition,
            terminator,
            parent_state,
            walk,
        );
    } else {
        terminator(tree, mutator_tree, target_id, mutator_id, parent_state, walk);
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn apply_repeat_while_to_children<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate<E>,
    terminator: Terminator<E, M>,
    parent_state: &E::Inherited,
    walk: &mut Walk<'_, E>,
) {
    let mut head = get_target(&tree.arena, target_id).first_child();
    if head.is_none() {
        return;
    }
    let state = own_state(tree, target_id, parent_state);
    // The descent is routed by kind, as every walk is: a child of
    // another kind than the repeated mutator is not tested, not mutated
    // and not descended into. The channel is not compared — the
    // descent broadcasts across channels on purpose.
    let kind = get_mutator(&mutator_tree.arena, mutator_id).get().key().kind;
    // The sibling link is read *before* recursing, because the
    // recursion may mutate the child it is handed and we still need
    // somewhere to go next.
    while let Some(head_id) = head {
        let child = get_target(&tree.arena, head_id);
        head = child.next_sibling();
        if child.get().kind() != kind {
            debug!("Child of another kind, not descending into it");
            continue;
        }
        debug!("Found child, recursing down sub-tree");
        repeat_while(
            tree,
            mutator_tree,
            head_id,
            mutator_id,
            condition,
            terminator,
            &state,
            walk,
        );
    }
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
    walk: &mut Walk<'_, E>,
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
    apply_recorded(tree, hit_id, walk, |hit| mutation.apply_to(hit, &state));
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
