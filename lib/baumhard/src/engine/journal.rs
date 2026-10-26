// SPDX-License-Identifier: MPL-2.0

//! Journals: an exact record of what a walk changed, which can undo
//! the change ([`Journal::revert`](crate::engine::journal::Journal::revert)),
//! redo it ([`Journal::replay`](crate::engine::journal::Journal::replay)) or
//! show it part-way ([`Journal::write_interpolated`](crate::engine::journal::Journal::write_interpolated)).
//!
//! # Who records
//!
//! A [`JournalBuilder`](crate::engine::journal::JournalBuilder) is handed to a journaled walk
//! ([`MutatorTree::apply_at_journaled`](crate::engine::tree::MutatorTree::apply_at_journaled))
//! or a journaled update ([`Tree::update_journaled`](crate::engine::tree::Tree::update_journaled)).
//! The **walker** records an element immediately before its first apply
//! to it within the builder: a clone of the element — its
//! *before-image* — and its position, `(parent, previous sibling)`.
//! Later applies to the same element in the same builder record
//! nothing. [`Mutation`](crate::engine::mutation::Mutation)s know
//! nothing about any of this and cannot opt out: the recording happens
//! in the one function every walker apply goes through. An element the
//! walk visits but applies nothing to is not recorded.
//!
//! # What closing does
//!
//! [`JournalBuilder::close`](crate::engine::journal::JournalBuilder::close)
//! reads every recorded element's *after* state and position from the
//! tree, and drops each entry whose element holds the
//! [same state](crate::engine::element::TreeElement::same_state) it
//! started with and whose position did not change. So
//! [`Journal::is_empty`](crate::engine::journal::Journal::is_empty) means
//! "nothing changed".
//!
//! # Positions
//!
//! An element whose key a walk changed is moved to its sorted place
//! among its siblings (the sorted-children invariant), so a journal
//! restores positions as well as values. Each moved element carries
//! two: where it stood **when the builder was created**, and where it
//! stands when the builder closes. The first is its position at its
//! first apply, unless an earlier move in the same builder had already
//! shifted it — taking a sibling out from in front of it, or putting
//! one there — in which case the builder kept its earlier position
//! when that move happened. Recording every position against the one
//! arrangement the builder started from is what makes the restore
//! exact however many applies the builder spans: a position taken at
//! a later moment would name a neighbor that has since moved.
//!
//! Restoring an arrangement detaches every moved element and puts
//! each back after its recorded previous sibling, in the order the
//! arrangement has them, so each one's previous sibling is in place
//! before it is. Elements the builder never moved keep their order
//! among themselves throughout — only moved elements move — so they
//! are the frame the moved ones are put back into. A moved element
//! that ends where it started, behind a sibling that is itself in
//! place, did not move at all, and is treated as unmoved.
//!
//! # Exactness
//!
//! `revert` and `replay` write recorded values and recorded positions,
//! and evaluate nothing: no [`Mutation::apply_to`](crate::engine::mutation::Mutation::apply_to),
//! no [`TreeElement::inherit`](crate::engine::element::TreeElement::inherit),
//! no key comparison. Both validate every node they refer to before
//! writing anything.
//!
//! Structural entries — [`Touch::Inserted`](crate::engine::journal::Touch::Inserted),
//! [`Touch::Removed`](crate::engine::journal::Touch::Removed),
//! [`Touch::Moved`](crate::engine::journal::Touch::Moved) — belong to the
//! structural commands, which no walk performs yet; no journal
//! produces them.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::interpolate::{Interpolable, Snap};
use crate::engine::siblings;
use crate::engine::tree::{Tree, TreeError};
use indextree::{Arena, NodeId};
use log::error;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::hash_map::Entry as MapEntry;

/// What a journal entry says happened to its node, as
/// [`Journal::touched`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Touch {
    /// The element's state changed, its position among its siblings
    /// changed (its key moved it), or both. The only touch a journal
    /// produces today.
    Field,
    /// The node and its subtree were inserted. Reserved for the
    /// structural commands; no journal produces it yet.
    Inserted,
    /// The node and its subtree were removed. Reserved for the
    /// structural commands; no journal produces it yet.
    Removed,
    /// The node was re-parented. Reserved for the structural commands;
    /// no journal produces it yet — a move among siblings caused by a
    /// key change is part of a [`Touch::Field`] entry.
    Moved,
}

/// A position among siblings: the parent, and the sibling immediately
/// before (`None` at the front of the row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct At {
    parent: NodeId,
    prev: Option<NodeId>,
}

impl At {
    /// Where `node` stands in `arena` now, or `None` for a node with
    /// no parent (the root). O(1).
    fn of<E>(arena: &Arena<E>, node: NodeId) -> Option<At> {
        let current = arena.get(node)?;
        Some(At {
            parent: current.parent()?,
            prev: current.previous_sibling(),
        })
    }
}

/// One element a builder has recorded: its before-image and its
/// position as of the builder's creation.
#[derive(Clone, Debug)]
struct Recorded<E> {
    node: NodeId,
    before: E,
    /// `None` for the root, which has no siblings and never moves.
    at: Option<At>,
    /// Whether the engine has moved the element among its siblings
    /// since it was recorded.
    moved: bool,
}

/// An open journal: what a journaled walk or update has recorded so
/// far. Hand it to any number of
/// [`MutatorTree::apply_at_journaled`](crate::engine::tree::MutatorTree::apply_at_journaled) and
/// [`Tree::update_journaled`](crate::engine::tree::Tree::update_journaled)
/// calls on **one** tree, then [`close`](Self::close) it against that
/// tree — or [`abandon`](Self::abandon) it to undo everything it
/// recorded.
///
/// **Contract:** the positions a builder records are exact only if the
/// tree changes through nothing but journaled calls on this builder
/// from its creation until it is closed. A plain [`Tree::update`] that
/// moves an element, an [`insert_child`](Tree::insert_child) or a
/// [`remove_subtree`](Tree::remove_subtree) in between changes a
/// neighbor's previous sibling without the builder noting it, and no
/// check can detect that afterwards: [`Journal::revert`] refuses stale
/// ids only.
///
/// A builder is all or nothing: when a journaled call on it fails, the
/// call reverts everything the builder recorded — the tree returns to
/// where it stood when the builder was created — and leaves the
/// builder empty. [`abandon`](Self::abandon) is the same step for a
/// failure the caller detects itself.
///
/// Cost: one entry per element applied to — a clone of the element and
/// a hash-map slot — never per element visited. A walk that changes no
/// key records no position beyond each entry's own.
#[derive(Clone, Debug)]
pub struct JournalBuilder<E: TreeElement> {
    recorded: Vec<Recorded<E>>,
    /// `NodeId` → index into `recorded`.
    index: FxHashMap<NodeId, usize>,
    /// Elements not yet recorded whose previous sibling a move has
    /// changed, each with its previous sibling as of the builder's
    /// creation. Consulted, and cleared, when the element is recorded.
    displaced: FxHashMap<NodeId, Option<NodeId>>,
}

/// Not derived: `derive(Default)` would demand `E: Default`.
impl<E: TreeElement> Default for JournalBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: TreeElement> JournalBuilder<E> {
    /// An empty builder. O(1); allocates nothing until something is
    /// recorded.
    pub fn new() -> Self {
        JournalBuilder {
            recorded: Vec::new(),
            index: FxHashMap::default(),
            displaced: FxHashMap::default(),
        }
    }

    /// How many elements the builder has recorded — one per element
    /// applied to, however many times. For the engine's tests. O(1).
    pub(in crate::engine) fn recorded_len(&self) -> usize {
        self.recorded.len()
    }

    /// The nodes the builder has recorded, oldest first. For the
    /// engine's tests. No allocation.
    pub(in crate::engine) fn recorded_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.recorded.iter().map(|recorded| recorded.node)
    }

    /// Record `node` before its first apply within this builder: a
    /// clone of the element and its position as of the builder's
    /// creation. A node already recorded records nothing, and neither
    /// does one not in `arena`.
    ///
    /// Cost: one hash lookup; on the first touch, one clone of the
    /// element and one `Vec` / hash-map slot.
    pub(in crate::engine) fn record(&mut self, arena: &Arena<E>, node: NodeId) {
        let MapEntry::Vacant(slot) = self.index.entry(node) else {
            return;
        };
        let Some(current) = arena.get(node) else {
            return;
        };
        let at = current.parent().map(|parent| At {
            parent,
            prev: match self.displaced.remove(&node) {
                Some(prev) => prev,
                None => current.previous_sibling(),
            },
        });
        slot.insert(self.recorded.len());
        self.recorded.push(Recorded {
            node,
            before: current.get().clone(),
            at,
            moved: false,
        });
    }

    /// Note that the engine is about to detach `node` to move it among
    /// its siblings: mark its entry moved, and keep the previous
    /// sibling of the node behind it, whose previous sibling the detach
    /// is about to change. Call before the detach.
    pub(in crate::engine) fn note_detach(&mut self, arena: &Arena<E>, node: NodeId) {
        match self.index.get(&node) {
            Some(&i) => self.recorded[i].moved = true,
            // Only an element whose key an apply changed is moved, and
            // every apply records first.
            None => {
                error!("engine: node {node} moved without a journal entry; its move cannot be reverted");
                debug_assert!(
                    false,
                    "engine invariant: node {node} moved without a journal entry"
                );
            }
        }
        if let Some(next) = arena.get(node).and_then(|n| n.next_sibling()) {
            self.displace(next, Some(node));
        }
    }

    /// Note that the engine is about to attach a detached node under
    /// `parent` right after `prev` (`None`: at the front): keep the
    /// previous sibling of the node it will land in front of, which the
    /// attach is about to change. Call before the attach.
    pub(in crate::engine) fn note_attach(&mut self, arena: &Arena<E>, parent: NodeId, prev: Option<NodeId>) {
        let follower = match prev {
            Some(prev) => arena.get(prev).and_then(|n| n.next_sibling()),
            None => arena.get(parent).and_then(|n| n.first_child()),
        };
        if let Some(follower) = follower {
            self.displace(follower, prev);
        }
    }

    /// `node`'s previous sibling is about to change from `prev`. If the
    /// node is not recorded yet and has not been displaced before,
    /// `prev` is its previous sibling as of the builder's creation:
    /// keep it. O(1).
    fn displace(&mut self, node: NodeId, prev: Option<NodeId>) {
        if !self.index.contains_key(&node) {
            self.displaced.entry(node).or_insert(prev);
        }
    }

    /// Close the builder against `tree` — the tree every journaled call
    /// on it applied to — into an immutable [`Journal`].
    ///
    /// Reads each recorded element's state and position from `tree` as
    /// its *after*, and drops every entry whose element holds the
    /// [same state](TreeElement::same_state) as its before-image and
    /// did not move. An entry whose node is no longer in `tree` cannot
    /// be read; it is dropped with an error logged.
    ///
    /// Cost: one clone of each recorded element and an O(entries) scan;
    /// only when an element moved, O(entries) flags and hash maps over
    /// the moved elements alone.
    pub fn close(self, tree: &Tree<E>) -> Journal<E> {
        let arena = &tree.arena;
        let mut read: Vec<(Recorded<E>, E, Option<At>)> = Vec::with_capacity(self.recorded.len());
        for recorded in self.recorded {
            let Some(current) = arena.get(recorded.node) else {
                error!(
                    "engine: journaled node {} left the tree before its journal closed; its entry is dropped",
                    recorded.node
                );
                continue;
            };
            let after = current.get().clone();
            let after_at = At::of(arena, recorded.node);
            read.push((recorded, after, after_at));
        }

        let displaced = out_of_place(&read);
        let mut entries = Vec::with_capacity(read.len());
        let mut index = FxHashMap::default();
        for (i, (recorded, after, after_at)) in read.into_iter().enumerate() {
            let out = displaced.get(i).copied().unwrap_or(false);
            let position = match (out, recorded.at, after_at) {
                (true, Some(before_at), Some(after_at)) => Some((before_at, after_at)),
                _ => None,
            };
            if position.is_none() && recorded.before.same_state(&after) {
                continue;
            }
            index.insert(recorded.node, entries.len());
            entries.push(Entry {
                node: recorded.node,
                before: recorded.before,
                after,
                position,
            });
        }
        let to_before = arrangement_order(&entries, |(before, _)| before);
        let to_after = arrangement_order(&entries, |(_, after)| after);
        Journal {
            entries,
            index,
            to_before,
            to_after,
        }
    }

    /// Undo everything this builder recorded, consuming it: `tree`
    /// returns to where it stood when the builder was created. The
    /// all-or-nothing step for a request that fails outside an engine
    /// call — a check its caller makes after the journaled calls have
    /// run — and the same step a failing journaled call takes itself
    /// ([`Self::unwind`]), so a request is undone by one function
    /// whoever detects the failure.
    ///
    /// `tree` must be the tree the builder's calls applied to, changed
    /// by nothing else since (see the contract on [`JournalBuilder`]). A
    /// revert that is refused even so is logged, not raised: there is
    /// nothing left for the caller to do with the builder.
    ///
    /// Cost: [`Self::close`] plus [`Journal::revert`].
    pub fn abandon(mut self, tree: &mut Tree<E>) {
        self.unwind(tree);
    }

    /// Revert everything this builder recorded and leave it empty: the
    /// all-or-nothing step a journaled call takes when it fails. A
    /// revert that is refused — which only a tree changed behind the
    /// builder's back can cause — is logged; the builder is left empty
    /// either way.
    pub(in crate::engine) fn unwind(&mut self, tree: &mut Tree<E>) {
        let journal = std::mem::take(self).close(tree);
        if let Err(e) = journal.revert(tree) {
            error!("engine: could not revert a failed journaled call: {e}");
        }
    }
}

/// For each recorded element read at close, whether its position
/// changed: whether a restore must move it.
///
/// An element the engine never moved stands where it stood, relative
/// to every other unmoved element. A moved element whose position
/// differs between the two arrangements changed. A moved element whose
/// recorded positions are equal — the same parent, behind the same
/// sibling — is in place if that sibling is (or it is first in its
/// row), so it is out of place exactly when the sibling it follows is:
/// that is propagated from the elements known to be out of place.
///
/// Returns one flag per element of `read`, or an empty `Vec` — every
/// flag `false` — when nothing moved.
///
/// Cost: an O(entries) scan for a moved element, which allocates
/// nothing; when one moved, O(entries) for the flags plus two hash maps
/// over the moved elements alone.
fn out_of_place<E>(read: &[(Recorded<E>, E, Option<At>)]) -> Vec<bool> {
    if !read.iter().any(|(recorded, _, _)| recorded.moved) {
        return Vec::new();
    }
    let mut out = vec![false; read.len()];
    let mut worklist: Vec<usize> = Vec::new();
    // Moved elements that ended where they started, by the sibling
    // they follow.
    let mut stayed: FxHashMap<(NodeId, NodeId), usize> = FxHashMap::default();
    for (i, (recorded, _, after_at)) in read.iter().enumerate() {
        if !recorded.moved {
            continue;
        }
        if recorded.at != *after_at {
            out[i] = true;
            worklist.push(i);
        } else if let Some(At {
            parent,
            prev: Some(prev),
        }) = recorded.at
        {
            stayed.insert((parent, prev), i);
        }
    }
    while let Some(i) = worklist.pop() {
        let (recorded, _, _) = &read[i];
        let Some(At { parent, .. }) = recorded.at else {
            continue;
        };
        if let Some(follower) = stayed.remove(&(parent, recorded.node)) {
            out[follower] = true;
            worklist.push(follower);
        }
    }
    out
}

/// The order in which to put the moved entries back so that each one's
/// previous sibling, in the arrangement `pick` selects, is in place
/// before it is: indices into `entries`, row by row, front to back.
///
/// An entry whose previous sibling is not itself moved (or which is
/// first in its row) heads a run; each run is followed through the
/// entries that come right after it.
///
/// Cost: O(moved entries) with a hash map and a hash set over them;
/// nothing when nothing moved.
fn arrangement_order<E>(entries: &[Entry<E>], pick: impl Fn(&(At, At)) -> &At) -> Vec<usize> {
    let moved: Vec<(usize, At)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| entry.position.as_ref().map(|position| (i, *pick(position))))
        .collect();
    if moved.is_empty() {
        return Vec::new();
    }
    let moved_nodes: FxHashSet<NodeId> = moved.iter().map(|&(i, _)| entries[i].node).collect();
    let mut follows: FxHashMap<(NodeId, NodeId), usize> = FxHashMap::default();
    let mut heads: Vec<usize> = Vec::new();
    for &(i, at) in &moved {
        match at.prev {
            Some(prev) if moved_nodes.contains(&prev) => {
                follows.insert((at.parent, prev), i);
            }
            _ => heads.push(i),
        }
    }
    let mut order = Vec::with_capacity(moved.len());
    for head in heads {
        let mut cursor = Some(head);
        while let Some(i) = cursor {
            order.push(i);
            let parent = moved_parent(entries, i, &pick);
            cursor = parent.and_then(|parent| follows.remove(&(parent, entries[i].node)));
        }
    }
    if order.len() != moved.len() {
        // A real arrangement has no cycle of previous siblings, so
        // every moved entry is reached from a head. Should one not be,
        // putting it back anyway beats leaving it detached.
        error!("engine: a journal's recorded positions form a cycle; restoring them in recorded order");
        for &(i, _) in &moved {
            if !order.contains(&i) {
                order.push(i);
            }
        }
    }
    order
}

/// The parent of moved entry `i` in the arrangement `pick` selects.
fn moved_parent<E>(entries: &[Entry<E>], i: usize, pick: &impl Fn(&(At, At)) -> &At) -> Option<NodeId> {
    entries[i].position.as_ref().map(|position| pick(position).parent)
}

/// One closed entry: an element whose state or position the journal
/// changed.
#[derive(Clone, Debug)]
struct Entry<E> {
    node: NodeId,
    before: E,
    after: E,
    /// `(before, after)` positions when the element's position changed;
    /// `None` when only its state did.
    position: Option<(At, At)>,
}

/// A closed journal: exactly what a builder's calls changed, as
/// before and after values per element. Immutable; made by
/// [`JournalBuilder::close`].
///
/// Undo is [`revert`](Self::revert) and redo is
/// [`replay`](Self::replay); [`touched`](Self::touched) names what
/// changed, for whatever has to follow the change. A journal belongs to
/// the tree its builder was used on; applying it to another tree is
/// refused only as far as a stale id can be detected.
#[derive(Clone, Debug)]
pub struct Journal<E: TreeElement> {
    entries: Vec<Entry<E>>,
    /// `NodeId` → index into `entries`.
    index: FxHashMap<NodeId, usize>,
    /// The moved entries, in the order that restores the arrangement
    /// before the journal.
    to_before: Vec<usize>,
    /// The moved entries, in the order that restores the arrangement
    /// after it.
    to_after: Vec<usize>,
}

impl<E: TreeElement> Journal<E> {
    /// Whether the journal records no change at all: every element its
    /// calls applied to holds the state it started with, in the place
    /// it started at. O(1).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every node the journal changed, with what happened to it, oldest
    /// entry first. Today every entry is a [`Touch::Field`]. No
    /// allocation.
    pub fn touched(&self) -> impl Iterator<Item = (NodeId, Touch)> + '_ {
        self.entries.iter().map(|entry| (entry.node, Touch::Field))
    }

    /// The element at `node` as it was before the journal, or `None`
    /// when the journal did not change it. O(1).
    pub fn before(&self, node: NodeId) -> Option<&E> {
        self.index.get(&node).map(|&i| &self.entries[i].before)
    }

    /// The element at `node` as the journal left it, or `None` when the
    /// journal did not change it. O(1).
    pub fn after(&self, node: NodeId) -> Option<&E> {
        self.index.get(&node).map(|&i| &self.entries[i].after)
    }

    /// Undo the journal: write every entry's before-image, newest entry
    /// first, then put every moved element back where it stood. `tree`
    /// must be the journal's tree as the journal left it — after the
    /// journaled calls, or after a [`replay`](Self::replay).
    ///
    /// Writes recorded values only: no mutation is applied and nothing
    /// is inherited or compared. Errors (nothing is written):
    /// [`TreeError::StaleNode`] for a node the journal names that is no
    /// longer in the tree, [`TreeError::NotAChild`] for a moved node or
    /// a recorded sibling no longer under its recorded parent.
    ///
    /// Invalidates the tree's cache. Cost: one clone per entry; for
    /// moved elements a detach and an O(depth) re-attach each.
    pub fn revert(&self, tree: &mut Tree<E>) -> Result<(), TreeError> {
        self.validate(tree)?;
        if self.entries.is_empty() {
            return Ok(());
        }
        for entry in self.entries.iter().rev() {
            write(&mut tree.arena, entry.node, entry.before.clone());
        }
        self.arrange(&mut tree.arena, &self.to_before, |(before, _)| before);
        tree.cache.invalidate();
        Ok(())
    }

    /// Redo the journal: write every entry's after-state, oldest entry
    /// first, then put every moved element where the journal left it.
    /// `tree` must be the journal's tree as it stood before the journal
    /// — before the journaled calls, or after a
    /// [`revert`](Self::revert).
    ///
    /// The same guarantees, errors and cost as [`Self::revert`].
    pub fn replay(&self, tree: &mut Tree<E>) -> Result<(), TreeError> {
        self.validate(tree)?;
        if self.entries.is_empty() {
            return Ok(());
        }
        for entry in &self.entries {
            write(&mut tree.arena, entry.node, entry.after.clone());
        }
        self.arrange(&mut tree.arena, &self.to_after, |(_, after)| after);
        tree.cache.invalidate();
        Ok(())
    }

    /// Check that every node the journal will write or move, and every
    /// sibling and parent it will place against, is in `tree` where the
    /// journal expects it. Reads only; O(entries).
    fn validate(&self, tree: &Tree<E>) -> Result<(), TreeError> {
        let arena = &tree.arena;
        for entry in &self.entries {
            let Some(node) = arena.get(entry.node) else {
                return Err(TreeError::StaleNode(entry.node));
            };
            let Some((before, after)) = entry.position else {
                continue;
            };
            for at in [before, after] {
                if arena.get(at.parent).is_none() {
                    return Err(TreeError::StaleNode(at.parent));
                }
                if node.parent() != Some(at.parent) {
                    return Err(TreeError::NotAChild {
                        parent: at.parent,
                        node: entry.node,
                    });
                }
                if let Some(prev) = at.prev {
                    let Some(sibling) = arena.get(prev) else {
                        return Err(TreeError::StaleNode(prev));
                    };
                    if sibling.parent() != Some(at.parent) {
                        return Err(TreeError::NotAChild {
                            parent: at.parent,
                            node: prev,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Put the moved entries in the arrangement `pick` selects: detach
    /// every one, then attach each after its recorded previous sibling,
    /// in `order` — the order that has each previous sibling in place
    /// first. Unmoved siblings never leave their rows. Nothing to do
    /// when nothing moved.
    fn arrange(&self, arena: &mut Arena<E>, order: &[usize], pick: impl Fn(&(At, At)) -> &At) {
        for &i in order {
            self.entries[i].node.detach(arena);
        }
        for &i in order {
            let entry = &self.entries[i];
            if let Some(position) = &entry.position {
                let at = pick(position);
                siblings::attach_at(arena, at.parent, entry.node, at.prev);
            }
        }
    }
}

impl<E: Interpolable> Journal<E> {
    /// Write the state between the journal's before and after at
    /// progress `t` into `tree`: each entry's element is
    /// [`Interpolable::interpolate`]d, and moved elements stand in the
    /// before or the after arrangement by the same `snap` rule that
    /// decides the element's non-interpolating fields — key included —
    /// so the rows stay sorted.
    ///
    /// `t` is clamped to `0..=1`, and a `NaN` reads as `0`. At `t = 0`
    /// the tree is left as [`revert`](Self::revert) leaves it, and at
    /// `t = 1` as [`replay`](Self::replay) does, whatever the snap.
    /// `tree` must be the journal's tree in the before arrangement, the
    /// after arrangement, or a previous `write_interpolated`'s.
    ///
    /// The same errors as [`Self::revert`], checked before anything is
    /// written. Invalidates the tree's cache. Cost: one
    /// `interpolate` per entry; for moved elements a detach and an
    /// O(depth) re-attach each.
    pub fn write_interpolated(&self, tree: &mut Tree<E>, t: f32, snap: Snap) -> Result<(), TreeError> {
        self.validate(tree)?;
        if self.entries.is_empty() {
            return Ok(());
        }
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        for entry in &self.entries {
            write(
                &mut tree.arena,
                entry.node,
                E::interpolate(&entry.before, &entry.after, t, snap),
            );
        }
        if snap.takes_after(t) {
            self.arrange(&mut tree.arena, &self.to_after, |(_, after)| after);
        } else {
            self.arrange(&mut tree.arena, &self.to_before, |(before, _)| before);
        }
        tree.cache.invalidate();
        Ok(())
    }
}

/// Overwrite the element at `node`, which the caller has validated.
fn write<E>(arena: &mut Arena<E>, node: NodeId, value: E) {
    if let Some(slot) = arena.get_mut(node) {
        *slot.get_mut() = value;
    }
}

/// Move `node` — whose key just changed — from its place among its
/// siblings to `prev`'s right (`None`: the front of `parent`'s row),
/// noting the move in `journal` when there is one. The one way the
/// engine moves an element among its siblings outside a batch.
///
/// Cost: a detach and an O(depth) re-attach, plus O(1) bookkeeping
/// with a journal.
pub(in crate::engine) fn move_among_siblings<E: TreeElement>(
    arena: &mut Arena<E>,
    parent: NodeId,
    node: NodeId,
    prev: Option<NodeId>,
    mut journal: Option<&mut JournalBuilder<E>>,
) {
    if let Some(journal) = journal.as_deref_mut() {
        journal.note_detach(arena, node);
    }
    node.detach(arena);
    if let Some(journal) = journal {
        journal.note_attach(arena, parent, prev);
    }
    siblings::attach_at(arena, parent, node, prev);
}
