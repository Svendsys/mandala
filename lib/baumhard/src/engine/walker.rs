// SPDX-License-Identifier: MPL-2.0

//! The walker behind [`MutatorTree::apply_to`](crate::engine::tree::MutatorTree::apply_to): aligns mutators to
//! target-tree elements by [`channel`](crate::engine::element::TreeElement::channel) and
//! dispatches each mutator's effect. Handles straight-line alignment,
//! [`Instruction::RepeatWhile`](crate::engine::mutator::Instruction::RepeatWhile) loop expansion (with the
//! [`Predicate`](crate::engine::predicate::Predicate) language), [`Instruction::MapChildren`](crate::engine::mutator::Instruction::MapChildren)'s positional
//! zip and [`Instruction::SpatialDescend`](crate::engine::mutator::Instruction::SpatialDescend)'s hit-test.
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
//! # Cache invalidation
//!
//! Every time the walker applies a mutator to a target it invalidates
//! the target tree's
//! [`TreeCache`](crate::engine::element::TreeCache) (one O(1) call),
//! so a caller that drives [`walk_tree_from`](crate::engine::walker::walk_tree_from()) directly sees no stale
//! derived state afterwards. Nothing inside a walk reads the cache.
//!
//! The entry points are [`walk_tree`](crate::engine::walker::walk_tree()) and [`walk_tree_from`](crate::engine::walker::walk_tree_from()).
//! [`repeat_while`](crate::engine::walker::repeat_while()) and [`default_terminator`](crate::engine::walker::default_terminator()) are `pub` too, so
//! mutator-DSL authors can compose custom terminators without forking
//! the walker; everything else in this module is internal.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::mutation::Mutation;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::tree::{MutatorTree, Tree};
use crate::util::geometry::aabb_contains;
use crate::util::ordered_vec2::OrderedVec2;
use indextree::{Arena, Node, NodeId};
use log::{debug, warn};

/// The continuation [`repeat_while`] calls when its predicate fails
/// on a target: `(tree, mutator_tree, target_id, mutator_id,
/// parent_state)`, with `parent_state` the inherited state of
/// `target_id`'s parent. [`default_terminator`] is the one the walker
/// uses.
pub type Terminator<E, M> =
    fn(&mut Tree<E>, &MutatorTree<E, M>, NodeId, NodeId, &<E as TreeElement>::Inherited);

/// Continuation called after a conditional loop
/// ([`Instruction::RepeatWhile`]) exits — it resumes the normal walk
/// from the mutator's children whose channel matches the current
/// target. `pub` so mutator authors can substitute custom terminators
/// when extending the walker; not a stable public API.
///
/// The after-mutations attached under `mutator_id` are treated as a
/// channel-sorted stream (same logic as the normal child-alignment
/// walk): arena order is not assumed to be ascending. Every
/// after-mutation whose channel equals the target's is dispatched via
/// [`walk_tree_from`], with the target's own `parent_state`; the scan
/// stops as soon as it passes the target's channel.
pub fn default_terminator<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
) {
    // When a conditional loop terminates, both the mutator and the
    // target are where the predicate failed: the target has not been
    // mutated (yet), and the mutator is one step behind.
    debug!("The Terminator has received a mission.");
    let t_chan = get_target(&mut tree.arena, target_id).get().channel();
    let after_mutators = collect_sorted_children(&mutator_tree.arena, mutator_id, |m| m.channel());
    for (after_id, after_chan) in after_mutators {
        if after_chan == t_chan {
            debug!("Next mutator matches the target, starting walk..");
            walk_tree_from(tree, mutator_tree, target_id, after_id, parent_state);
        } else if after_chan > t_chan {
            debug!("Next mutator channel is higher than target channel, ending branch..");
            break;
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
/// aligned children. `target_id` and `mutator_id` must belong to the
/// arenas that back `tree` and `mutator_tree` respectively;
/// `parent_state` is the inherited state of `target_id`'s parent.
/// Cost: O(sum of matching (target, mutator) pairs) — every pairwise
/// match is one apply; pruned branches are free.
pub fn walk_tree_from<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
) {
    let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();

    match mutator {
        Mutator::Single { .. } | Mutator::Macro { .. } => {
            debug!("Processing Delta Node...");
            apply_if_matching_channel(tree, target_id, mutator, parent_state);
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
            if mutation.is_some() {
                debug!("This instruction node has a Delta..");
                apply_if_matching_channel(tree, target_id, mutator, parent_state);
            }
            process_instruction_node(
                tree,
                mutator_tree,
                target_id,
                mutator_id,
                instruction,
                parent_state,
            );
            return;
        }
    }
    align_child_walks(tree, mutator_tree, target_id, mutator_id, parent_state);
}

/// Apply `mutator` to `target_id` when their channels match, and
/// invalidate the tree's cache if it did.
#[inline]
fn apply_if_matching_channel<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    target_id: NodeId,
    mutator: &Mutator<E, M>,
    parent_state: &E::Inherited,
) {
    let target = get_target(&mut tree.arena, target_id).get_mut();
    if mutator.channel() == target.channel() {
        debug!("Delta and target channel match, applying..");
        mutator.apply_to(target, parent_state);
        tree.cache.invalidate();
    } else {
        debug!("Delta mutator channel does not match target channel.");
    }
}

/// The inherited state `target_id` hands its children: its own
/// [`TreeElement::inherit`] of `parent_state`, read after its own
/// mutations. O(1) plus the element type's `inherit`.
#[inline]
fn own_state<E: TreeElement>(tree: &Tree<E>, target_id: NodeId, parent_state: &E::Inherited) -> E::Inherited {
    tree.arena
        .get(target_id)
        .expect("walker invariant: target NodeId must belong to tree.arena")
        .get()
        .inherit(parent_state)
}

#[inline]
fn process_instruction_node<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    instruction: &Instruction<E>,
    parent_state: &E::Inherited,
) {
    match instruction {
        Instruction::RepeatWhile(condition) => {
            let mutator = get_mutator(&mutator_tree.arena, mutator_id);
            let target = get_target(&mut tree.arena, target_id);
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
            compare_apply_repeat_while(tree, mutator_tree, target_id, mutator_id, condition, &state)
        }
        Instruction::SpatialDescend(point) => {
            spatial_descend(tree, mutator_tree, target_id, mutator_id, point, parent_state);
        }
        Instruction::MapChildren => {
            zip_map_children(tree, mutator_tree, target_id, mutator_id, parent_state);
        }
    };
}

/// Walk the children of `target_parent_id` and `mutator_parent_id` as
/// channel-sorted streams, applying [`repeat_while`] for every
/// matching (target, mutator) pair. `state` is the inherited state of
/// `target_parent_id` itself — the parent of every target this
/// function hands on.
///
/// Mirrors [`align_child_walks`]: arena order is **not** assumed to
/// be channel-ascending, so both sibling rows are collected and
/// sorted before the merge walk. The sorted merge advances only the
/// mutator when `m_chan < t_chan` and only the target when
/// `m_chan > t_chan`, preserving broadcast semantics (one mutator
/// may apply to multiple consecutive targets sharing its channel).
///
/// Cost: O(n log n) per sibling row for the sort, where `n` is the
/// sibling count under one parent, plus one `Vec` per row.
fn compare_apply_repeat_while<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_parent_id: NodeId,
    mutator_parent_id: NodeId,
    condition: &Predicate<E>,
    state: &E::Inherited,
) {
    let mutator_children = collect_sorted_children(&mutator_tree.arena, mutator_parent_id, |m| m.channel());
    if mutator_children.is_empty() {
        debug!("RepeatWhile mutator has no children - nothing to align.");
        return;
    }
    let target_children = collect_sorted_children(&tree.arena, target_parent_id, |t| t.channel());

    let mut t_idx = 0usize;
    for (m_id, m_chan) in mutator_children.iter().copied() {
        while t_idx < target_children.len() {
            let (t_id, t_chan) = target_children[t_idx];
            if t_chan == m_chan {
                t_idx += 1;
                repeat_while(
                    tree,
                    mutator_tree,
                    t_id,
                    m_id,
                    condition,
                    default_terminator::<E, M>,
                    state,
                );
            } else if t_chan > m_chan {
                debug!(
                    "Target channel {} exceeds mutator channel {}, advancing to next mutator.",
                    t_chan, m_chan
                );
                break;
            } else {
                t_idx += 1;
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
fn get_target<E: TreeElement>(arena: &mut Arena<E>, id: NodeId) -> &mut Node<E> {
    arena
        .get_mut(id)
        .expect("walker invariant: target NodeId must belong to tree.arena")
}

/// Take the children of the mutator and of the target, and start a
/// walk for each matching channel pair. A mutator reaches every
/// target on its channel that the merge has not already passed: the
/// first mutator on a channel consumes that channel's targets, so a
/// later sibling mutator on the same channel reaches none of them.
/// `parent_state` is the inherited state of `target_id`'s parent; the
/// children receive `target_id`'s own state.
///
/// See also [`zip_map_children`] — the opt-out alternative that pairs
/// children by sibling position (zip) instead of by channel, for
/// mutations that need per-index targeting.
///
/// # Channel-sorted merge walk
///
/// The walker treats the target's and mutator's children as sorted
/// streams keyed by [`channel`](TreeElement::channel) and applies
/// each mutator child to every target child sharing its channel. The
/// arena order of children is **not** assumed to be channel-ascending
/// — children are collected into local `Vec`s and sorted by channel
/// before the merge walk. This removes a long-standing fragility
/// where the walker's `t_chan > m_chan` break could prematurely skip
/// matches when arena order disagreed with channel order (see
/// `console_mutator_round_trips_to_fresh_build` for a fixture that
/// exercises a non-ascending sibling row).
///
/// Cost: O(n log n) per sibling row for the sort, where `n` is the
/// sibling count under one parent, plus one `Vec` per row. Sibling
/// counts are small in practice (single-digit), so the sort is
/// effectively free next to the per-pair `walk_tree_from` recursion.
#[inline]
fn align_child_walks<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
) {
    debug!(
        "Aligning children of target node {} and mutator node {}.",
        target_id, mutator_id
    );
    let mutator_children = collect_sorted_children(&mutator_tree.arena, mutator_id, |m| m.channel());
    if mutator_children.is_empty() {
        debug!("Mutator has no children - nothing to align.");
        return;
    }
    let target_children = collect_sorted_children(&tree.arena, target_id, |t| t.channel());
    let state = own_state(tree, target_id, parent_state);

    let mut t_idx = 0usize;
    for (m_id, m_chan) in mutator_children.iter().copied() {
        while t_idx < target_children.len() {
            let (t_id, t_chan) = target_children[t_idx];
            if t_chan == m_chan {
                t_idx += 1;
                walk_tree_from(tree, mutator_tree, t_id, m_id, &state);
            } else if t_chan > m_chan {
                debug!(
                    "Target channel {} exceeds mutator channel {}, advancing to next mutator.",
                    t_chan, m_chan
                );
                break;
            } else {
                t_idx += 1;
            }
        }
    }
}

/// Collect direct children of `parent` paired with their channel,
/// sorted ascending by channel. Stable sort so siblings with identical
/// channels keep their arena-relative order — pairs the channel-merge
/// walk against itself deterministically.
fn collect_sorted_children<T>(
    arena: &Arena<T>,
    parent: NodeId,
    channel_of: impl Fn(&T) -> usize,
) -> Vec<(NodeId, usize)> {
    let mut out = Vec::new();
    let mut cur = arena.get(parent).and_then(|n| n.first_child());
    while let Some(id) = cur {
        if let Some(node) = arena.get(id) {
            out.push((id, channel_of(node.get())));
            cur = node.next_sibling();
        } else {
            break;
        }
    }
    out.sort_by_key(|&(_, ch)| ch);
    out
}

/// Zip the mutator's direct children against the target's direct
/// children by sibling position — the alternative to
/// [`align_child_walks`] for consumers that need per-index targeting
/// independent of channel semantics (e.g. size-aware layouts where
/// every target child sits on the same broadcast channel).
///
/// For each pair up to `min(mutator_children_len,
/// target_children_len)`, the mutator's own effect is
/// **force-applied** to its paired target — bypassing the
/// channel-match check that [`apply_if_matching_channel`] normally
/// enforces. That bypass is the whole point: channels on the paired
/// children are broadcast tags, and MapChildren's job is to ignore
/// them at the pairing site.
///
/// After the force-apply, the mutator's subtree descends against the
/// target's subtree through the standard path:
/// - Single/Macro/Void → [`align_child_walks`] (channel-aware).
/// - Instruction → [`process_instruction_node`], which re-dispatches
///   into whichever instruction body the child carries (including a
///   nested MapChildren, which will zip one level deeper).
///
/// Excess children on either side are silently dropped with a single
/// `debug!` line at termination. No allocation inside the loop (§B7).
/// Graceful no-op on empty children on either side.
///
/// The *outer* instruction's own attached mutation is already applied
/// to the current target by [`walk_tree_from`] before this function is
/// called (same precedent as [`compare_apply_repeat_while`]) — this
/// function handles only the descent into paired children, which
/// receive `target_id`'s own inherited state.
#[inline]
fn zip_map_children<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    parent_state: &E::Inherited,
) {
    let mut option_mutator_child = get_mutator(&mutator_tree.arena, mutator_id).first_child();
    let mut option_target_child = get_target(&mut tree.arena, target_id).first_child();
    let state = own_state(tree, target_id, parent_state);

    let mut paired: usize = 0;
    while let (Some(mutator_child_id), Some(target_child_id)) = (option_mutator_child, option_target_child) {
        // Look up the next-siblings *before* the force-apply that
        // takes `&mut tree.arena`, so the read-only borrows end
        // cleanly.
        let next_mutator = mutator_tree
            .arena
            .get(mutator_child_id)
            .and_then(|n| n.next_sibling());
        let next_target = tree.arena.get(target_child_id).and_then(|n| n.next_sibling());

        // Force-apply the mutator to its paired target. The mutator is
        // borrowed from `mutator_tree`, which the dispatch below only
        // reads, so a nested instruction is handed on by reference.
        let mutator = get_mutator(&mutator_tree.arena, mutator_child_id).get();
        mutator.apply_to(get_target(&mut tree.arena, target_child_id).get_mut(), &state);
        tree.cache.invalidate();
        match mutator {
            Mutator::Instruction { instruction, .. } => {
                // Nested instruction: dispatch at the paired target.
                // Matches `walk_tree_from`'s post-apply path for
                // Instruction (`process_instruction_node` + early
                // return — no align_child_walks).
                process_instruction_node(
                    tree,
                    mutator_tree,
                    target_child_id,
                    mutator_child_id,
                    instruction,
                    &state,
                );
            }
            Mutator::Single { .. } | Mutator::Macro { .. } | Mutator::Void { .. } => {
                // Descend via channel-based align at the next level
                // down. A user who wants the deeper level to also zip
                // nests MapChildren inside.
                align_child_walks(tree, mutator_tree, target_child_id, mutator_child_id, &state);
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
/// `parent_state` is the inherited state of `target_id`'s parent.
///
/// This is the engine behind [`Instruction::RepeatWhile`]; it is
/// exposed so mutator authors can supply a custom `terminator` without
/// forking the walker (see [`default_terminator`] for the default
/// continuation). The `terminator` receives the same tree pair, the
/// current target/mutator ids and `parent_state`.
pub fn repeat_while<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate<E>,
    terminator: Terminator<E, M>,
    parent_state: &E::Inherited,
) {
    let condition_matches = condition.test(get_target(&mut tree.arena, target_id).get());
    if condition_matches {
        debug!(
            "Condition is met, applying mutator {} to target {}",
            mutator_id, target_id
        );
        let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();
        mutator.apply_to(get_target(&mut tree.arena, target_id).get_mut(), parent_state);
        tree.cache.invalidate();
        apply_repeat_while_to_children(
            tree,
            mutator_tree,
            target_id,
            mutator_id,
            condition,
            terminator,
            parent_state,
        );
    } else {
        terminator(tree, mutator_tree, target_id, mutator_id, parent_state);
    }
}

#[inline]
fn apply_repeat_while_to_children<E: TreeElement, M: Mutation<E>>(
    tree: &mut Tree<E>,
    mutator_tree: &MutatorTree<E, M>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate<E>,
    terminator: Terminator<E, M>,
    parent_state: &E::Inherited,
) {
    let mut head = get_target(&mut tree.arena, target_id).first_child();
    if head.is_none() {
        return;
    }
    let state = own_state(tree, target_id, parent_state);
    // The sibling link is read *before* recursing, because the
    // recursion may mutate the child it is handed and we still need
    // somewhere to go next.
    while let Some(head_id) = head {
        debug!("Found child, recursing down sub-tree");
        head = get_target(&mut tree.arena, head_id).next_sibling();
        repeat_while(
            tree,
            mutator_tree,
            head_id,
            mutator_id,
            condition,
            terminator,
            &state,
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
///    channel — the spatial match overrides channel alignment —
///    with the hit's parent's inherited state, folded down the path
///    from `target_id`.
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
    mutation.apply_to(get_target(&mut tree.arena, hit_id).get_mut(), &state);
    tree.cache.invalidate();
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
