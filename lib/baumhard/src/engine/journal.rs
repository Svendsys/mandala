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
//! The walk's [structural commands](crate::engine::structure) record
//! the same way: a removed or moved node is recorded before it is
//! detached, and a node a command creates — the root of an inserted
//! subtree, the second part of a split — is recorded as it was created,
//! with no position, since it was not in the tree.
//!
//! # What closing does
//!
//! [`JournalBuilder::close`](crate::engine::journal::JournalBuilder::close)
//! reads every recorded element's *after* state and position from the
//! tree, and drops each entry whose element holds the
//! [same state](crate::engine::element::TreeElement::same_state) it
//! started with and whose position did not change. So
//! [`Journal::is_empty`](crate::engine::journal::Journal::is_empty) means
//! "nothing changed" — nothing in the tree: a change inside a subtree
//! the builder created and removed again is kept, so that the journal
//! restores that subtree consistently, but no one can see it.
//!
//! # Positions
//!
//! An element whose key a walk changed is moved to its sorted place
//! among its siblings (the sorted-children invariant), and a structural
//! command inserts, removes and re-parents nodes, so a journal restores
//! positions as well as values. Each moved element carries two: where
//! it stood **when the builder was created**, and where it stands when
//! the builder closes. Either can be *nowhere* — not in the tree: an
//! inserted node has only a close position, a removed one only a start
//! position, and a re-parented one has one in each parent.
//!
//! The start position is the element's position at its first apply,
//! unless an earlier change in the same builder had already shifted it
//! — taking a sibling out from in front of it, or putting one there —
//! in which case the builder kept its earlier position when that change
//! happened. Recording every position against the one arrangement the
//! builder started from is what makes the restore exact however many
//! calls the builder spans: a position taken at a later moment would
//! name a neighbor that has since moved. (A node the builder created
//! starts out of the tree, and the nodes below it where it created them.)
//!
//! Restoring an arrangement detaches every moved element and puts
//! each back after its recorded previous sibling, in the order the
//! arrangement has them, so each one's previous sibling is in place
//! before it is; an element the arrangement has out of the tree stays
//! detached. Elements the builder never moved keep their parent and
//! their order among themselves throughout — only moved elements move —
//! so they are the frame the moved ones are put back into. A moved
//! element that ends where it started, behind a sibling that is itself
//! in place, did not move at all, and is treated as unmoved.
//!
//! Restoring arrangement, not history, is what lets structural and key
//! changes mix in one builder: positions carry no time, so there is no
//! order of moves to replay — only two arrangements to restore.
//!
//! # Detached subtrees
//!
//! A subtree out of the tree in one of a journal's arrangements stays
//! allocated, so the other arrangement can put back the same `NodeId`s.
//! The journal owns it: [`Journal::discard`](crate::engine::journal::Journal::discard)
//! frees what the journal holds detached, and nothing else frees it.
//!
//! # Exactness
//!
//! `revert` and `replay` write recorded values and recorded positions,
//! and evaluate nothing: no [`Mutation::apply_to`](crate::engine::mutation::Mutation::apply_to),
//! no [`TreeElement::inherit`](crate::engine::element::TreeElement::inherit),
//! no key comparison. Both validate every node they refer to before
//! writing anything.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::interpolate::{Interpolable, Snap};
use crate::engine::siblings;
use crate::engine::tree::{Tree, TreeError};
use indextree::{Arena, NodeId};
use log::error;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;
use std::collections::hash_map::Entry as MapEntry;

/// What happened to a node, as [`Journal::touched`] reports it: how its
/// place in the tree differs between the journal's two arrangements.
///
/// A touch names the change, not the direction: after a
/// [`revert`](Journal::revert) an `Inserted` node is out of the tree
/// again, so a reader of the tree finds the node's current place there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Touch {
    /// The element's state changed, its position among its siblings
    /// changed (a key change or a move within its row), or both. It is
    /// in the tree before and after, under the same parent.
    Field,
    /// The node is in the tree after the journal and was not before:
    /// inserted, or under a subtree that was.
    Inserted,
    /// The node was in the tree before the journal and is not after:
    /// removed, or under a subtree that was.
    Removed,
    /// The node is in the tree before and after, under another parent:
    /// re-parented, its subtree with it.
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
    /// no parent: the root, or the top of a detached subtree. O(1).
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
    /// `None` for a node with no parent when the builder was created:
    /// the root, which never moves, or a node the builder created.
    at: Option<At>,
    /// Whether the engine has moved the element — among its siblings,
    /// to another parent, into the tree or out of it — since it was
    /// recorded.
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
/// where it stood when the builder was created, and every node the
/// builder's structural commands created is freed — and leaves the
/// builder empty. [`abandon`](Self::abandon) is the same step for a
/// failure the caller detects itself.
///
/// A builder is closed or abandoned, never dropped: a dropped builder
/// leaves every node its commands created and every subtree they removed
/// allocated, out of the tree, owned by nothing.
///
/// Cost: one entry per element applied to — a clone of the element and
/// a hash-map slot — never per element visited. A walk that changes no
/// key and no structure records no position beyond each entry's own.
#[derive(Clone, Debug)]
pub struct JournalBuilder<E: TreeElement> {
    recorded: Vec<Recorded<E>>,
    /// `NodeId` → index into `recorded`.
    index: FxHashMap<NodeId, usize>,
    /// Elements not yet recorded whose previous sibling a move has
    /// changed, each with its previous sibling as of the builder's
    /// creation. Consulted, and cleared, when the element is recorded.
    displaced: FxHashMap<NodeId, Option<NodeId>>,
    /// The tree's [version](Tree::version) when this builder's first
    /// call began, and the fresh one its calls hold the tree at; `None`
    /// before any call. What the closed journal checks its restores
    /// against.
    versions: Option<(u64, u64)>,
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
            versions: None,
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

    /// Note that a journaled call on this builder is about to change
    /// `tree`: on the builder's first call, keep the tree's version as
    /// the one its journal starts from and move the tree to a fresh
    /// version; on a later call, hold the tree at that version. Every
    /// journaled entry point calls this first. O(1).
    pub(in crate::engine) fn begin(&mut self, tree: &mut Tree<E>) {
        let working = match self.versions {
            Some((_, working)) => working,
            None => {
                let working = tree.next_version;
                tree.next_version += 1;
                self.versions = Some((tree.version.get(), working));
                working
            }
        };
        tree.version.set(working);
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

    /// Record `node`, which a structural command has just created — and
    /// may already have put in place, since a new node goes in through
    /// an insert that climbs nothing: as it was created, with no start
    /// position — it was not in the tree when the builder was created —
    /// and as moved, since it enters the tree. The nodes below it are
    /// not recorded: they stand where it was created with them.
    ///
    /// Cost: one clone of the element and one `Vec` / hash-map slot.
    pub(in crate::engine) fn record_created(&mut self, arena: &Arena<E>, node: NodeId) {
        let MapEntry::Vacant(slot) = self.index.entry(node) else {
            return;
        };
        let Some(current) = arena.get(node) else {
            return;
        };
        slot.insert(self.recorded.len());
        self.recorded.push(Recorded {
            node,
            before: current.get().clone(),
            at: None,
            moved: true,
        });
    }

    /// Note that the engine is about to detach `node` — to move it
    /// among its siblings, to another parent or out of the tree: mark
    /// its entry moved, and keep the previous sibling of the node
    /// behind it, whose previous sibling the detach is about to change.
    /// Call before the detach.
    pub(in crate::engine) fn note_detach(&mut self, arena: &Arena<E>, node: NodeId) {
        match self.index.get(&node) {
            Some(&i) => self.recorded[i].moved = true,
            // Only a node the engine recorded first is ever moved.
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
    /// did not move. An entry whose node is no longer in `tree`'s arena
    /// cannot be read; it is dropped with an error logged.
    ///
    /// A journal that closes [empty](Journal::is_empty) has a single
    /// arrangement, so closing it puts `tree`'s version stamp back
    /// where the builder's first call found it — the one change `close`
    /// makes, through a `Cell` — and the journal holds that stamp on
    /// both sides. Discarded, it leaves
    /// nothing a restore of an earlier journal would refuse as
    /// [out of order](TreeError::OutOfOrder).
    ///
    /// Cost: one clone of each recorded element and an O(entries) scan;
    /// only when an element moved, O(entries) flags and hash maps over
    /// the moved elements alone; only when a node entered, left or
    /// changed parent, the [touches](Journal::touched) of every node of
    /// each subtree that entered or left, with each node above an entry
    /// climbed through once per side — never once per entry.
    #[must_use = "a journal owns detached subtrees: discard it, do not drop it"]
    pub fn close(self, tree: &Tree<E>) -> Journal<E> {
        let arena = &tree.arena;
        let current = tree.version.get();
        let (before_version, after_version) = self.versions.unwrap_or((current, current));
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
        let out = |i: usize| displaced.get(i).copied().unwrap_or(false);
        let keep: Vec<bool> = read
            .iter()
            .enumerate()
            .map(|(i, (recorded, after, _))| out(i) || !recorded.before.same_state(after))
            .collect();

        // What the journal holds detached on either side: every node
        // other than the root with no parent at that end.
        let mut detached_before = Vec::new();
        let mut detached_after = Vec::new();
        for (recorded, _, after_at) in &read {
            if recorded.node == tree.root {
                continue;
            }
            if recorded.at.is_none() {
                detached_before.push(recorded.node);
            }
            if after_at.is_none() {
                detached_after.push(recorded.node);
            }
        }
        let touched = structural_touches(arena, tree.root, &read, &out, &keep);

        let mut entries = Vec::with_capacity(read.len());
        let mut index = FxHashMap::default();
        for (i, (recorded, after, after_at)) in read.into_iter().enumerate() {
            if !keep[i] {
                continue;
            }
            let position = out(i).then_some(Positions {
                before: recorded.at,
                after: after_at,
            });
            index.insert(recorded.node, entries.len());
            entries.push(Entry {
                node: recorded.node,
                before: recorded.before,
                after,
                position,
            });
        }
        let out = entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| entry.position.map(|_| i))
            .collect();
        let to_before = arrangement_order(&entries, Positions::before);
        let to_after = arrangement_order(&entries, Positions::after);
        // An empty journal has one arrangement: the tree goes back to the
        // stamp it started from, which both sides then hold.
        let empty = match &touched {
            None => entries.is_empty(),
            Some(touched) => touched.is_empty(),
        };
        let after_version = if empty {
            tree.version.set(before_version);
            before_version
        } else {
            after_version
        };
        Journal {
            entries,
            index,
            out,
            to_before,
            to_after,
            touched,
            detached_before,
            detached_after,
            side: Cell::new(Side::After),
            before_version,
            after_version,
        }
    }

    /// Undo everything this builder recorded, consuming it: `tree`
    /// returns to where it stood when the builder was created, and every
    /// node the builder's structural commands created is freed. The
    /// all-or-nothing step for a request that fails outside an engine
    /// call — a check its caller makes after the journaled calls have
    /// run — and the same step a failing journaled call
    /// ([`MutatorTree::apply_at_journaled`](crate::engine::tree::MutatorTree::apply_at_journaled),
    /// [`Tree::update_journaled`]) takes itself, so a request is undone
    /// by one function whoever detects the failure.
    ///
    /// `tree` must be the tree the builder's calls applied to, changed
    /// by nothing else since (see the contract on [`JournalBuilder`]). A
    /// revert that is refused even so is logged, not raised: there is
    /// nothing left for the caller to do with the builder, and nothing
    /// is freed.
    ///
    /// Cost: [`Self::close`] plus [`Journal::revert`] plus
    /// [`Journal::discard`].
    pub fn abandon(mut self, tree: &mut Tree<E>) {
        self.unwind(tree);
    }

    /// Revert everything this builder recorded, free what its commands
    /// created, and leave it empty: the all-or-nothing step a journaled
    /// call takes when it fails. A revert that is refused — which only a
    /// tree changed behind the builder's back can cause — is logged, and
    /// then nothing is freed; the builder is left empty either way.
    pub(in crate::engine) fn unwind(&mut self, tree: &mut Tree<E>) {
        let journal = std::mem::take(self).close(tree);
        match journal.revert(tree) {
            Ok(()) => journal.discard(tree),
            Err(e) => error!("engine: could not revert a failed journaled call: {e}"),
        }
    }
}

/// For each recorded element read at close, whether its position
/// changed: whether a restore must move it.
///
/// An element the engine never moved stands where it stood, relative
/// to every other unmoved element. A moved element whose position
/// differs between the two arrangements changed — a node that entered
/// or left the tree included, whose position is nowhere at one end. A
/// moved element whose recorded positions are equal — the same parent,
/// behind the same sibling, or out of the tree at both ends — is in
/// place if that sibling is (or it is first in its row, or out of the
/// tree), so it is out of place exactly when the sibling it follows is:
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

/// The [touches](Journal::touched) of a journal whose nodes entered or
/// left the tree or changed parent — `None` for one where none did,
/// whose every entry is a [`Touch::Field`].
///
/// Each node's touch is read from whether it is in the tree — under
/// the root — in the arrangement before and in the arrangement after:
/// [`Touch::Inserted`] for a node in only the after, [`Touch::Removed`]
/// for one in only the before, and for one in both, [`Touch::Moved`]
/// when its parent changed and [`Touch::Field`] when only its state or
/// its place in its row did. A node in neither is outside what anyone
/// can see — created and removed again — and is not reported.
///
/// **Order.** The entries are taken oldest first, and every node is
/// named once:
/// - A node that entered the tree is named with the whole subtree that
///   entered with it. That subtree's top is the highest node above it,
///   in the arrangement after, that was not in the tree before; the
///   top's subtree is named in the after arrangement's pre-order, at the
///   first entry that leads to it.
/// - A node that left is named the same way with the subtree that left
///   with it, in the **before** arrangement's pre-order, rebuilt from the
///   recorded positions.
/// - A node in the tree at both ends is named at its own entry, unless
///   one of those subtrees named it first.
///
/// So a node that entered or left is never named before an ancestor
/// that entered or left with it, and each subtree that entered or left
/// is named in one piece.
///
/// `read` holds every element recorded, `out(i)` whether element `i`
/// changed position, and `keep[i]` whether it is an entry of the
/// journal.
///
/// Cost: an O(entries) scan; when something entered, left or changed
/// parent, hash maps over the recorded nodes and the nodes looked at,
/// each node above an entry climbed through once per arrangement (the
/// memoized [`Presence`] climbs), each subtree that entered or left
/// walked once, and the rows of the before arrangement rebuilt only
/// where a moved node stood in them.
fn structural_touches<E>(
    arena: &Arena<E>,
    root: NodeId,
    read: &[(Recorded<E>, E, Option<At>)],
    out: &impl Fn(usize) -> bool,
    keep: &[bool],
) -> Option<Vec<(NodeId, Touch)>> {
    let structural = read.iter().any(|(recorded, _, after_at)| {
        recorded.node != root
            && !matches!((recorded.at, after_at), (Some(b), Some(a)) if b.parent == a.parent)
    });
    if !structural {
        return None;
    }
    // A node's parent before the journal is its recorded one if it
    // changed position, and its parent now otherwise: nothing else
    // changes parent. The nodes that changed position, by the parent
    // they stood under before, are what the before rows are rebuilt
    // from.
    let mut before_parent: FxHashMap<NodeId, Option<NodeId>> = FxHashMap::default();
    let mut moved_from: FxHashMap<NodeId, Vec<(NodeId, Option<NodeId>)>> = FxHashMap::default();
    let mut entry_of: FxHashMap<NodeId, usize> = FxHashMap::default();
    for (i, (recorded, _, _)) in read.iter().enumerate() {
        if out(i) {
            before_parent.insert(recorded.node, recorded.at.map(|at| at.parent));
            if let Some(at) = recorded.at {
                moved_from
                    .entry(at.parent)
                    .or_default()
                    .push((recorded.node, at.prev));
            }
        }
        if keep[i] {
            entry_of.insert(recorded.node, i);
        }
    }
    let mut presence = Presence {
        arena,
        root,
        before_parent,
        before: FxHashMap::default(),
        after: FxHashMap::default(),
        entered: FxHashMap::default(),
        left: FxHashMap::default(),
        chain: Vec::new(),
        top_chain: Vec::new(),
    };
    // The touch of a node in the tree at both ends: what its own entry
    // says, or none when it has no entry.
    let own = |node: NodeId| -> Option<Touch> {
        let &i = entry_of.get(&node)?;
        let (recorded, _, after_at) = &read[i];
        let reparented = out(i) && recorded.at.map(|at| at.parent) != after_at.map(|at| at.parent);
        Some(if reparented { Touch::Moved } else { Touch::Field })
    };
    let mut seen: FxHashSet<NodeId> = FxHashSet::default();
    let mut touched = Vec::new();
    let mut pending: Vec<NodeId> = Vec::new();
    let mut row: Vec<NodeId> = Vec::new();
    let mut follows: FxHashMap<Option<NodeId>, NodeId> = FxHashMap::default();
    let mut name = |member: NodeId, presence: &mut Presence<'_, E>, touched: &mut Vec<(NodeId, Touch)>| {
        if !seen.insert(member) {
            return;
        }
        let touch = match (presence.before(member), presence.after(member)) {
            (false, true) => Some(Touch::Inserted),
            (true, false) => Some(Touch::Removed),
            (true, true) => own(member),
            (false, false) => None,
        };
        touched.extend(touch.map(|touch| (member, touch)));
    };
    let mut named_tops: FxHashSet<NodeId> = FxHashSet::default();
    for (i, (recorded, _, _)) in read.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let node = recorded.node;
        let (before, after) = (presence.before(node), presence.after(node));
        let mut in_a_subtree = false;
        if after {
            if let Some(top) = presence.top_entered(node) {
                in_a_subtree = true;
                if named_tops.insert(top) {
                    for member in top.descendants(arena) {
                        name(member, &mut presence, &mut touched);
                    }
                }
            }
        }
        if before {
            if let Some(top) = presence.top_left(node) {
                in_a_subtree = true;
                if named_tops.insert(top) {
                    pending.clear();
                    pending.push(top);
                    while let Some(member) = pending.pop() {
                        name(member, &mut presence, &mut touched);
                        children_before(
                            arena,
                            member,
                            &presence.before_parent,
                            &moved_from,
                            &mut follows,
                            &mut row,
                        );
                        pending.extend(row.iter().rev());
                    }
                }
            }
        }
        if before && after && !in_a_subtree {
            name(node, &mut presence, &mut touched);
        }
    }
    Some(touched)
}

/// `parent`'s children in the arrangement before a journal, front to
/// back, into `row`: its children now that kept their position, in the
/// order they keep, with every node that stood under `parent` before
/// and changed position put back after its recorded previous sibling —
/// the restore [`Journal::revert`] makes, read without writing.
/// `before_parent` names the nodes that changed position; `moved_from`
/// holds them by the parent they stood under before; `follows` is
/// scratch, cleared first.
///
/// The moved nodes are indexed once by the sibling they follow, and the
/// row is then built in one pass: each kept child, then the run of
/// moved nodes that follow it. Returns the number of steps taken — one
/// per moved node indexed, per kept child and per moved node placed —
/// so that its cost can be checked: O(row + moved), and no work list.
pub(in crate::engine) fn children_before<E>(
    arena: &Arena<E>,
    parent: NodeId,
    before_parent: &FxHashMap<NodeId, Option<NodeId>>,
    moved_from: &FxHashMap<NodeId, Vec<(NodeId, Option<NodeId>)>>,
    follows: &mut FxHashMap<Option<NodeId>, NodeId>,
    row: &mut Vec<NodeId>,
) -> usize {
    row.clear();
    follows.clear();
    let mut steps = 0;
    let moved: &[(NodeId, Option<NodeId>)] = moved_from.get(&parent).map_or(&[], Vec::as_slice);
    for &(node, prev) in moved {
        follows.insert(prev, node);
        steps += 1;
    }
    place_run(follows, None, row, &mut steps);
    for child in parent.children(arena) {
        if before_parent.contains_key(&child) {
            continue;
        }
        steps += 1;
        row.push(child);
        place_run(follows, Some(child), row, &mut steps);
    }
    if !follows.is_empty() {
        // A real arrangement chains every moved node behind a sibling
        // in it. Should one not be, listing it anyway beats losing it.
        error!("engine: a journal's recorded positions under {parent} do not chain");
        let left: FxHashSet<NodeId> = follows.values().copied().collect();
        row.extend(
            moved
                .iter()
                .map(|&(node, _)| node)
                .filter(|node| left.contains(node)),
        );
        follows.clear();
    }
    steps
}

/// Append to `row` the run of moved nodes behind `prev` in `follows`,
/// each found by the one before it, taking them out of `follows`.
fn place_run(
    follows: &mut FxHashMap<Option<NodeId>, NodeId>,
    prev: Option<NodeId>,
    row: &mut Vec<NodeId>,
    steps: &mut usize,
) {
    let mut cursor = follows.remove(&prev);
    while let Some(node) = cursor {
        row.push(node);
        *steps += 1;
        cursor = follows.remove(&Some(node));
    }
}

/// Whether nodes are in the tree before and after a journal, memoized:
/// a node is in the tree when its chain of parents reaches the root.
/// After is read from the arena at close; before follows each node's
/// parent before the journal.
struct Presence<'a, E> {
    arena: &'a Arena<E>,
    root: NodeId,
    /// The parent before the journal of every node whose position
    /// changed; every other node's is its parent now.
    before_parent: FxHashMap<NodeId, Option<NodeId>>,
    before: FxHashMap<NodeId, bool>,
    after: FxHashMap<NodeId, bool>,
    /// [`Self::top_entered`] of every node a question has climbed
    /// through.
    entered: FxHashMap<NodeId, Option<NodeId>>,
    /// [`Self::top_left`] of every node a question has climbed through.
    left: FxHashMap<NodeId, Option<NodeId>>,
    /// Scratch for [`resolve`]'s chain, kept so a close allocates it
    /// once.
    chain: Vec<NodeId>,
    /// Scratch for [`Self::top`]'s chain, apart from `chain` because
    /// `top` asks `before` and `after` while it holds its own.
    top_chain: Vec<NodeId>,
}

impl<E> Presence<'_, E> {
    /// `id`'s parent before the journal.
    fn parent_before(&self, id: NodeId) -> Option<NodeId> {
        match self.before_parent.get(&id) {
            Some(&parent) => parent,
            None => self.arena.get(id).and_then(|n| n.parent()),
        }
    }

    /// Whether `node` was in the tree before the journal.
    fn before(&mut self, node: NodeId) -> bool {
        let arena = self.arena;
        let before_parent = &self.before_parent;
        resolve(
            self.root,
            arena.len(),
            &mut self.before,
            &mut self.chain,
            node,
            |id| match before_parent.get(&id) {
                Some(&parent) => parent,
                None => arena.get(id).and_then(|n| n.parent()),
            },
        )
    }

    /// Whether `node` is in the tree as the journal left it.
    fn after(&mut self, node: NodeId) -> bool {
        let arena = self.arena;
        resolve(
            self.root,
            arena.len(),
            &mut self.after,
            &mut self.chain,
            node,
            |id| arena.get(id).and_then(|n| n.parent()),
        )
    }

    /// The top of the subtree that entered the tree with `node`, which
    /// is in the tree after the journal: the highest node on its chain
    /// of parents after — `node` included — that was not in the tree
    /// before. `None` when `node` and every node above it were.
    /// Amortized O(1) over a close: see [`Self::top`].
    fn top_entered(&mut self, node: NodeId) -> Option<NodeId> {
        self.top(node, Side::After)
    }

    /// The top of the subtree that left the tree with `node`, which was
    /// in the tree before the journal: the highest node on its chain of
    /// parents before — `node` included — that is not in the tree after.
    /// `None` when `node` and every node above it still are. Amortized
    /// O(1) over a close: see [`Self::top`].
    fn top_left(&mut self, node: NodeId) -> Option<NodeId> {
        self.top(node, Side::Before)
    }

    /// [`Self::top_entered`] (`side` [`Side::After`]: the chain of
    /// parents after, nodes not in the tree before) or
    /// [`Self::top_left`] (`side` [`Side::Before`]: the chain before,
    /// nodes not in the tree after), memoized for every node on the
    /// chain: a node's answer is its parent's when that has one, and
    /// otherwise the node itself if it is out on the other side. The
    /// climb stops at the first node already answered, so over a whole
    /// close each node is climbed through once per side, however many
    /// entries sit below it — never once per entry (§20 rule 11). A
    /// chain longer than the arena is a cycle no real arrangement has,
    /// and is cut there.
    fn top(&mut self, node: NodeId, side: Side) -> Option<NodeId> {
        let mut chain = std::mem::take(&mut self.top_chain);
        chain.clear();
        let mut cursor = Some(node);
        let mut above = None;
        while let Some(id) = cursor {
            let memo = match side {
                Side::After => &self.entered,
                Side::Before => &self.left,
            };
            if let Some(&known) = memo.get(&id) {
                above = known;
                break;
            }
            if chain.len() > self.arena.len() {
                error!("engine: a journal's recorded parents form a cycle at node {id}");
                break;
            }
            chain.push(id);
            #[cfg(test)]
            tests_support::count_presence_step();
            cursor = match side {
                Side::After => self.arena.get(id).and_then(|n| n.parent()),
                Side::Before => self.parent_before(id),
            };
        }
        for &id in chain.iter().rev() {
            if above.is_none() {
                let out = match side {
                    Side::After => !self.before(id),
                    Side::Before => !self.after(id),
                };
                if out {
                    above = Some(id);
                }
            }
            match side {
                Side::After => self.entered.insert(id, above),
                Side::Before => self.left.insert(id, above),
            };
        }
        self.top_chain = chain;
        above_of(
            match side {
                Side::After => &self.entered,
                Side::Before => &self.left,
            },
            node,
        )
    }
}

/// A memoized top's answer for `node`. O(1).
fn above_of(memo: &FxHashMap<NodeId, Option<NodeId>>, node: NodeId) -> Option<NodeId> {
    memo.get(&node).copied().flatten()
}

/// Whether the chain of `parent_of` from `node` reaches `root`, with
/// every node on the chain memoized in `memo`; `chain` is scratch,
/// cleared first. A chain longer than the arena's `limit` nodes is a
/// cycle — no real arrangement has one — and answers `false` with an
/// error logged. O(chain up to a memoized node); no allocation once
/// `chain` has grown to the depth.
fn resolve(
    root: NodeId,
    limit: usize,
    memo: &mut FxHashMap<NodeId, bool>,
    chain: &mut Vec<NodeId>,
    node: NodeId,
    parent_of: impl Fn(NodeId) -> Option<NodeId>,
) -> bool {
    chain.clear();
    let mut cursor = Some(node);
    let answer = loop {
        let Some(id) = cursor else {
            break false;
        };
        if id == root {
            break true;
        }
        if let Some(&known) = memo.get(&id) {
            break known;
        }
        if chain.len() > limit {
            error!("engine: a journal's recorded parents form a cycle at node {id}");
            break false;
        }
        chain.push(id);
        cursor = parent_of(id);
    };
    for &id in chain.iter() {
        memo.insert(id, answer);
    }
    answer
}

/// The order in which to put the moved entries back so that each one's
/// previous sibling, in the arrangement `pick` selects, is in place
/// before it is: indices into `entries`, row by row, front to back. An
/// entry that arrangement has out of the tree is not put anywhere, and
/// is not in the order.
///
/// An entry whose previous sibling is not itself moved (or which is
/// first in its row) heads a run; each run is followed through the
/// entries that come right after it.
///
/// Cost: O(moved entries) with a hash map and a hash set over them;
/// nothing when nothing moved.
fn arrangement_order<E>(entries: &[Entry<E>], pick: fn(&Positions) -> Option<At>) -> Vec<usize> {
    let moved: Vec<(usize, At)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| entry.position.as_ref().and_then(pick).map(|at| (i, at)))
        .collect();
    if moved.is_empty() {
        return Vec::new();
    }
    let moved_nodes: FxHashSet<NodeId> = moved.iter().map(|&(i, _)| entries[i].node).collect();
    let mut follows: FxHashMap<(NodeId, NodeId), usize> = FxHashMap::default();
    let mut parent_of: FxHashMap<usize, NodeId> = FxHashMap::default();
    let mut heads: Vec<usize> = Vec::new();
    for &(i, at) in &moved {
        parent_of.insert(i, at.parent);
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
            cursor = parent_of
                .get(&i)
                .and_then(|&parent| follows.remove(&(parent, entries[i].node)));
        }
    }
    if order.len() != moved.len() {
        // A real arrangement has no cycle of previous siblings, so
        // every moved entry is reached from a head. Should one not be,
        // putting it back anyway beats leaving it detached.
        error!("engine: a journal's recorded positions form a cycle; restoring them in recorded order");
        let placed: FxHashSet<usize> = order.iter().copied().collect();
        for &(i, _) in &moved {
            if !placed.contains(&i) {
                order.push(i);
            }
        }
    }
    order
}

/// Where a moved element stands in the arrangement before the journal
/// and in the one after it; `None` where it is out of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Positions {
    before: Option<At>,
    after: Option<At>,
}

impl Positions {
    /// The position before the journal.
    fn before(&self) -> Option<At> {
        self.before
    }

    /// The position after the journal.
    fn after(&self) -> Option<At> {
        self.after
    }
}

/// One closed entry: an element whose state or position the journal
/// changed.
#[derive(Clone, Debug)]
struct Entry<E> {
    node: NodeId,
    before: E,
    after: E,
    /// Its positions when its position changed; `None` when only its
    /// state did.
    position: Option<Positions>,
}

/// Which of its two arrangements a journal last wrote to the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// The arrangement before the journal: after a revert, or a
    /// [`Journal::write_interpolated`] that took the before side.
    Before,
    /// The arrangement after it: as closed, after a replay, or a
    /// `write_interpolated` that took the after side.
    After,
}

/// A closed journal: exactly what a builder's calls changed, as
/// before and after values and positions per element. Made by
/// [`JournalBuilder::close`]; what it records never changes.
///
/// Undo is [`revert`](Self::revert) and redo is
/// [`replay`](Self::replay); [`touched`](Self::touched) names what
/// changed, for whatever has to follow the change. A journal belongs to
/// the tree its builder was used on; applying it to another tree is
/// refused only as far as a stale id can be detected.
///
/// A journal **owns** the subtrees its structural commands took out of
/// the tree: the ones it removed, while it is applied, and the ones it
/// inserted, while it is reverted. They stay allocated so that the same
/// `NodeId`s come back, until [`discard`](Self::discard) frees them. A
/// journal dropped without `discard` leaves them allocated, unreachable,
/// for as long as the tree lives.
///
/// A clone shares that ownership: both copies name the same detached
/// subtrees, and only one may free them. Keep one journal per change —
/// a history holds journals, never clones of them — and discard each
/// once, after its last write. (`discard` frees only what is still
/// detached, so a clone discarded on the wrong side frees nothing that
/// is in the tree; but the subtree a copy still needs to restore is gone
/// from under it.)
#[derive(Clone, Debug)]
pub struct Journal<E: TreeElement> {
    entries: Vec<Entry<E>>,
    /// `NodeId` → index into `entries`.
    index: FxHashMap<NodeId, usize>,
    /// The entries whose position changed: the ones a restore detaches.
    out: Vec<usize>,
    /// The moved entries in the tree before the journal, in the order
    /// that restores that arrangement.
    to_before: Vec<usize>,
    /// The moved entries in the tree after it, in the order that
    /// restores that arrangement.
    to_after: Vec<usize>,
    /// Every touch, when a node entered, left or changed parent; `None`
    /// when every entry is a [`Touch::Field`].
    touched: Option<Vec<(NodeId, Touch)>>,
    /// The tops of the subtrees out of the tree before the journal.
    detached_before: Vec<NodeId>,
    /// The tops of the subtrees out of the tree after it.
    detached_after: Vec<NodeId>,
    /// The arrangement last written, which decides what
    /// [`Self::discard`] frees.
    side: Cell<Side>,
    /// The tree's [version](Tree::version) in the arrangement before the
    /// journal, and in the one after it: a restore is accepted only from
    /// one of the two ([`TreeError::OutOfOrder`]).
    before_version: u64,
    after_version: u64,
}

impl<E: TreeElement> Journal<E> {
    /// Whether the journal records no change to the tree: every node in
    /// the tree before or after it holds the state it started with, in
    /// the place it started at — exactly when [`Self::touched`] names
    /// nothing. O(1).
    ///
    /// A subtree its builder's commands created and removed again was
    /// never in the tree, so what was done inside it is no change; but
    /// the journal owns that subtree, so a caller that drops an empty
    /// journal frees it with [`Self::discard`] all the same.
    pub fn is_empty(&self) -> bool {
        match &self.touched {
            None => self.entries.is_empty(),
            Some(touched) => touched.is_empty(),
        }
    }

    /// Every node the journal changed, with what happened to it
    /// ([`Touch`]), each once.
    ///
    /// The entries are taken oldest first. A node that entered the tree
    /// is named with the whole subtree that entered with it, from that
    /// subtree's top down, in the pre-order of the arrangement after the
    /// journal; a node that left, with the whole subtree that left with
    /// it, in the pre-order of the arrangement before. Each node there
    /// carries its own touch — the same one, unless a node was moved
    /// into that subtree from elsewhere. So a node that entered or left
    /// is never named before an ancestor that entered or left with it,
    /// and each such subtree is named in one piece, however many calls
    /// of the builder made it. Any other node is named at its own entry.
    /// A node that was neither in the tree before nor after — created
    /// and removed again — is not reported.
    ///
    /// No allocation: a journal with no structural change reports its
    /// entries as they are, and one with a structural change computed
    /// its touches when it closed.
    pub fn touched(&self) -> impl Iterator<Item = (NodeId, Touch)> + '_ {
        let fields = match self.touched {
            None => Some(self.entries.iter().map(|entry| (entry.node, Touch::Field))),
            Some(_) => None,
        };
        self.touched
            .iter()
            .flatten()
            .copied()
            .chain(fields.into_iter().flatten())
    }

    /// The element at `node` as it was before the journal, or `None`
    /// when the journal did not change it. For a node the journal's
    /// commands created, the element as it was created. O(1).
    pub fn before(&self, node: NodeId) -> Option<&E> {
        self.index.get(&node).map(|&i| &self.entries[i].before)
    }

    /// The element at `node` as the journal left it, or `None` when the
    /// journal did not change it. O(1).
    pub fn after(&self, node: NodeId) -> Option<&E> {
        self.index.get(&node).map(|&i| &self.entries[i].after)
    }

    /// Where `node` hung before the journal and where it hangs after it,
    /// as `(parent before, parent after)` — `None` on a side where it is
    /// out of the tree — or `None` when its parent did not change. A
    /// removed node answers `(Some(old parent), None)`, an inserted one
    /// `(None, Some(new parent))`, a re-parented one both.
    ///
    /// Only the node that itself changed parent answers: the nodes below
    /// the top of a subtree that entered or left keep their parent, and
    /// the arena has it. So the parent a removed subtree left — the row
    /// a removal changed, which the arena no longer links to — is read
    /// here for its top. O(1); no allocation.
    pub fn parents(&self, node: NodeId) -> Option<(Option<NodeId>, Option<NodeId>)> {
        let position = self.entries[*self.index.get(&node)?].position?;
        let parents = (
            position.before.map(|at| at.parent),
            position.after.map(|at| at.parent),
        );
        (parents.0 != parents.1).then_some(parents)
    }

    /// Undo the journal: write every entry's before-image, newest entry
    /// first, then put every moved element back where it stood — a node
    /// the journal inserted is detached again, and one it removed is
    /// attached again, with the same ids. `tree` must be the journal's
    /// tree as the journal left it — after the journaled calls, or after
    /// a [`replay`](Self::replay).
    ///
    /// Journals restore in their history's order: undo newest first,
    /// redo oldest first. A journal recorded after this one and still
    /// applied, or one recorded before it and still undone, has changed
    /// the tree from either arrangement this journal knows, and its
    /// touches no longer say which nodes it puts in or out of the tree.
    /// So that order is checked, in every build and in O(1) (the tree's
    /// version stamp), before anything is written. A structural change
    /// made without a journal ends the history the same way: every
    /// journal recorded before it is refused afterwards.
    ///
    /// Writes recorded values only: no mutation is applied and nothing
    /// is inherited or compared, and nothing is freed. Errors (nothing
    /// is written), checked in this order: [`TreeError::StaleNode`] for
    /// a node the journal names that is no longer in the arena,
    /// [`TreeError::NotAChild`] for a moved node under neither of its
    /// recorded parents, or a recorded sibling no longer under its
    /// recorded parent; then [`TreeError::OutOfOrder`] for a tree in
    /// neither of the journal's arrangements.
    ///
    /// Invalidates the tree's cache. Cost: one clone per entry; for
    /// moved elements a detach and a re-attach each: a same-row one for
    /// an element that keeps its parent (O(1) once indextree offers it,
    /// ruling R16 on #282), and an O(depth) one — the cycle check of a
    /// re-parent — for an element that changes parent, enters or leaves.
    pub fn revert(&self, tree: &mut Tree<E>) -> Result<(), TreeError> {
        self.validate(tree)?;
        self.in_order(tree)?;
        self.side.set(Side::Before);
        tree.version.set(self.before_version);
        if self.entries.is_empty() {
            return Ok(());
        }
        for entry in self.entries.iter().rev() {
            write(&mut tree.arena, entry.node, entry.before.clone());
        }
        self.arrange(&mut tree.arena, &self.to_before, Positions::before);
        self.mark(tree, Side::Before);
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
        self.in_order(tree)?;
        self.side.set(Side::After);
        tree.version.set(self.after_version);
        if self.entries.is_empty() {
            return Ok(());
        }
        for entry in &self.entries {
            write(&mut tree.arena, entry.node, entry.after.clone());
        }
        self.arrange(&mut tree.arena, &self.to_after, Positions::after);
        self.mark(tree, Side::After);
        tree.cache.invalidate();
        Ok(())
    }

    /// Free every subtree this journal holds out of the tree, consuming
    /// it: the subtrees it removed when it was last written forward
    /// (closed, [`replay`](Self::replay)ed, or written past the snap
    /// point by [`write_interpolated`](Self::write_interpolated)), the
    /// ones it inserted when it was last [`revert`](Self::revert)ed. A
    /// subtree is freed only if it is still detached — no parent, and
    /// not the root. Every id in a freed subtree becomes stale; nothing
    /// in the tree changes.
    ///
    /// The step for a journal leaving the history: the oldest entry
    /// falling off the end (applied), or a redo branch being cleared
    /// (reverted). A subtree the journal took out of the tree is held by
    /// it alone — no walk reaches it, and no other journal can put it
    /// back — so freeing it here leaves no id that anything could still
    /// restore. One that a later journal took out is that journal's, and
    /// this one does not free it.
    ///
    /// Invalidates the tree's cache when it frees anything. Cost:
    /// O(subtrees held) plus O(nodes freed).
    pub fn discard(self, tree: &mut Tree<E>) {
        let held = match self.side.get() {
            Side::Before => &self.detached_before,
            Side::After => &self.detached_after,
        };
        let mut freed = false;
        for &node in held {
            let detached = tree.arena.get(node).is_some_and(|n| n.parent().is_none());
            if detached && node != tree.root {
                for member in node.descendants(&tree.arena) {
                    tree.detached.remove(&member);
                }
                node.remove_subtree(&mut tree.arena);
                freed = true;
            }
        }
        if freed {
            tree.cache.invalidate();
        }
    }

    /// Mark in `tree`'s set of detached nodes which nodes are out of the
    /// tree in the arrangement `side`, now written: a node the touches
    /// name [`Touch::Inserted`] is in after the journal and out before
    /// it, a [`Touch::Removed`] one the other way round, and no other
    /// node changes whether it is in the tree. The touches name every
    /// element of each subtree that entered or left, so the set stays
    /// exact, and [`Tree::contains`] O(1).
    ///
    /// Cost: one hash write per element that entered or left — the
    /// touches every consumer of the change walks anyway; nothing for a
    /// journal with no structural change.
    fn mark(&self, tree: &mut Tree<E>, side: Side) {
        let Some(touched) = &self.touched else {
            return;
        };
        for &(node, touch) in touched {
            let out = match (touch, side) {
                (Touch::Inserted, Side::Before) | (Touch::Removed, Side::After) => true,
                (Touch::Inserted, Side::After) | (Touch::Removed, Side::Before) => false,
                (Touch::Field | Touch::Moved, _) => continue,
            };
            if out {
                tree.detached.insert(node);
            } else {
                tree.detached.remove(&node);
            }
        }
    }

    /// Whether `tree` is in one of this journal's two arrangements, by
    /// its [version](Tree::version): the one the journal started from
    /// or the one it left. Any other version means a journal recorded
    /// after this one is still applied, or one before it still undone —
    /// a restore out of the history's order, which would write
    /// positions against an arrangement the journal never saw and mark
    /// nodes in or out of the tree wrongly. O(1), in every build.
    fn in_order(&self, tree: &Tree<E>) -> Result<(), TreeError> {
        let version = tree.version.get();
        if version == self.before_version || version == self.after_version {
            Ok(())
        } else {
            Err(TreeError::OutOfOrder)
        }
    }

    /// Whether entry `node` changed position. O(1).
    fn is_out(&self, node: NodeId) -> bool {
        self.index
            .get(&node)
            .is_some_and(|&i| self.entries[i].position.is_some())
    }

    /// Check that every node the journal will write or move, and every
    /// sibling and parent it will place against, is in `tree` where the
    /// journal expects it: a moved node under one of its two recorded
    /// parents (or detached, where one end has it out of the tree), and
    /// each recorded sibling that does not move itself under the parent
    /// it was recorded under. Reads only; O(entries).
    fn validate(&self, tree: &Tree<E>) -> Result<(), TreeError> {
        let arena = &tree.arena;
        for entry in &self.entries {
            let Some(node) = arena.get(entry.node) else {
                return Err(TreeError::StaleNode(entry.node));
            };
            let Some(position) = entry.position else {
                continue;
            };
            let parents = [
                position.before.map(|at| at.parent),
                position.after.map(|at| at.parent),
            ];
            if !parents.contains(&node.parent()) {
                let Some(parent) = parents.into_iter().flatten().next() else {
                    return Err(TreeError::StaleNode(entry.node));
                };
                return Err(TreeError::NotAChild {
                    parent,
                    node: entry.node,
                });
            }
            for at in [position.before, position.after].into_iter().flatten() {
                if arena.get(at.parent).is_none() {
                    return Err(TreeError::StaleNode(at.parent));
                }
                if let Some(prev) = at.prev {
                    let Some(sibling) = arena.get(prev) else {
                        return Err(TreeError::StaleNode(prev));
                    };
                    if !self.is_out(prev) && sibling.parent() != Some(at.parent) {
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
    /// every one, then attach each that arrangement has in the tree
    /// after its recorded previous sibling, in `order` — the order that
    /// has each previous sibling in place first. Unmoved nodes never
    /// leave their rows. Nothing to do when nothing moved.
    ///
    /// An entry under the same parent in both arrangements only changes
    /// its place in its row, and goes back through
    /// [`siblings::reattach_in_row`]; one that changes parent, enters or
    /// leaves the tree is a re-parent ([`siblings::attach_at`]).
    fn arrange(&self, arena: &mut Arena<E>, order: &[usize], pick: fn(&Positions) -> Option<At>) {
        for &i in &self.out {
            self.entries[i].node.detach(arena);
        }
        for &i in order {
            let entry = &self.entries[i];
            let Some(position) = entry.position.as_ref() else {
                continue;
            };
            let Some(at) = pick(position) else {
                continue;
            };
            let same_row = matches!(
                (position.before, position.after),
                (Some(before), Some(after)) if before.parent == after.parent
            );
            if same_row {
                siblings::reattach_in_row(arena, at.parent, entry.node, at.prev);
            } else {
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
    /// so the rows stay sorted. Structure snaps with them: with
    /// [`Snap::Start`] the journal's inserts, removals and moves are in
    /// place for every `t > 0`, with [`Snap::End`] only at `t = 1`.
    ///
    /// `t` is clamped to `0..=1`, and a `NaN` reads as `0`. At `t = 0`
    /// the tree is left as [`revert`](Self::revert) leaves it, and at
    /// `t = 1` as [`replay`](Self::replay) does, whatever the snap.
    /// `tree` must be the journal's tree in the before arrangement, the
    /// after arrangement, or a previous `write_interpolated`'s.
    ///
    /// The same errors as [`Self::revert`], checked before anything is
    /// written. Invalidates the tree's cache. Cost: one
    /// `interpolate` per entry; for moved elements the
    /// detach and re-attach [`Self::revert`] describes.
    pub fn write_interpolated(&self, tree: &mut Tree<E>, t: f32, snap: Snap) -> Result<(), TreeError> {
        self.validate(tree)?;
        self.in_order(tree)?;
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let after = snap.takes_after(t);
        self.side.set(if after { Side::After } else { Side::Before });
        tree.version.set(if after {
            self.after_version
        } else {
            self.before_version
        });
        if self.entries.is_empty() {
            return Ok(());
        }
        for entry in &self.entries {
            write(
                &mut tree.arena,
                entry.node,
                E::interpolate(&entry.before, &entry.after, t, snap),
            );
        }
        if after {
            self.arrange(&mut tree.arena, &self.to_after, Positions::after);
            self.mark(tree, Side::After);
        } else {
            self.arrange(&mut tree.arena, &self.to_before, Positions::before);
            self.mark(tree, Side::Before);
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
/// Cost: a detach and a same-row re-attach
/// ([`siblings::reattach_in_row`]), plus O(1) bookkeeping with a
/// journal.
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
    siblings::reattach_in_row(arena, parent, node, prev);
}

/// What the tests read of a journal's close: how many steps the climbs
/// of [`Presence`] have taken on this thread. Compiled for tests only.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::cell::Cell;

    thread_local! {
        static PRESENCE_STEPS: Cell<usize> = const { Cell::new(0) };
    }

    /// One more node climbed through.
    pub(super) fn count_presence_step() {
        PRESENCE_STEPS.with(|cell| cell.set(cell.get() + 1));
    }

    /// The nodes every close on this thread has climbed through so far.
    pub(crate) fn presence_steps() -> usize {
        PRESENCE_STEPS.with(Cell::get)
    }
}
