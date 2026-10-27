// SPDX-License-Identifier: MPL-2.0

//! The two arenas a walk aligns. [`Tree<E>`](crate::engine::tree::Tree) wraps an
//! `indextree::Arena` of elements for O(1) child iteration and O(1)
//! node access, plus the element type's per-tree
//! [`TreeCache`](crate::engine::element::TreeCache);
//! [`MutatorTree<E, M>`](crate::engine::tree::MutatorTree) mirrors that shape but carries
//! [`Mutator`](crate::engine::mutator::Mutator)s instead of elements. The two meet in
//! [`MutatorTree::apply_to`](crate::engine::tree::MutatorTree::apply_to) and
//! [`MutatorTree::apply_at`](crate::engine::tree::MutatorTree::apply_at), which hand both arenas to the
//! [walk](crate::engine::walker). Everything here is mutation-first
//! per §B2: changing a field is a mutator applied in place, never a
//! rebuilt arena.
//!
//! # The sorted-children invariant
//!
//! Both arenas keep every parent's children in ascending
//! [`BranchKey`](crate::engine::key::BranchKey) order, children with equal keys in the order they were
//! placed in. The walker relies on it to merge a mutator row against a
//! target row in step, so both arenas are private, and every change
//! that could break the invariant goes through an API that keeps it:
//! [`Tree::insert_child`](crate::engine::tree::Tree::insert_child) and
//! [`MutatorTree::insert_child`](crate::engine::tree::MutatorTree::insert_child) place a new
//! node by its key, [`Tree::update`](crate::engine::tree::Tree::update) moves an element whose key it
//! changed, a walk moves the elements whose key it changed once
//! it is over, and the walk's [structural commands](crate::engine::structure)
//! — which insert, remove and move nodes after the walk — place every
//! node within the run of its key.
//! [`Tree::check_sorted`](crate::engine::tree::Tree::check_sorted) and
//! [`MutatorTree::check_sorted`](crate::engine::tree::MutatorTree::check_sorted)
//! verify it.
//!
//! # Journaled changes
//!
//! [`MutatorTree::apply_at_journaled`](crate::engine::tree::MutatorTree::apply_at_journaled) and
//! [`Tree::update_journaled`](crate::engine::tree::Tree::update_journaled) are the walk and the
//! update recorded in a [`JournalBuilder`](crate::engine::journal::JournalBuilder), for a tree that
//! keeps history: the closed [`Journal`](crate::engine::journal::Journal) reverts and replays
//! them exactly, structure included. A subtree a journaled walk removes
//! stays allocated, out of the tree, until its journal is
//! [discarded](crate::engine::journal::Journal::discard).

use crate::engine::anchor::{apply_anchored, Anchors};
use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::journal::{move_among_siblings, JournalBuilder};
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::{Mutator, MutatorType};
use crate::engine::siblings;
use crate::engine::walker::{run_walk, walk_anchor};
use indextree::{Arena, Children, Descendants, Node, NodeId};
use log::error;
use std::fmt;

/// Where [`Tree::insert_child`] / [`MutatorTree::insert_child`] puts a
/// new child **within the run of siblings whose key equals its own**.
/// The key decides the run; the placement only orders the new child
/// among equal keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Before every sibling with an equal key.
    First,
    /// After every sibling with an equal key.
    Last,
    /// Immediately before the named sibling, which must be a child of
    /// the same parent with an equal key.
    Before(NodeId),
    /// Immediately after the named sibling, which must be a child of
    /// the same parent with an equal key.
    After(NodeId),
}

/// Why a tree operation was refused. Every refusal leaves the tree
/// exactly as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// The id names no node of this tree: it was removed, or its slot
    /// has been reused since (indextree's per-slot stamp no longer
    /// matches).
    StaleNode(NodeId),
    /// A [`Placement::Before`] / [`Placement::After`] named a sibling
    /// whose key differs from the new child's: the placement would put
    /// the child outside the run of its own key.
    PlacementCrossesKey,
    /// A [`Placement::Before`] / [`Placement::After`] named a node that
    /// is not a child of the parent being inserted under.
    NotAChild {
        /// The parent the insertion was under.
        parent: NodeId,
        /// The node the placement named.
        node: NodeId,
    },
    /// [`Tree::remove_subtree`] or a
    /// [`StructOp::Remove`](crate::engine::structure::StructOp::Remove)
    /// was asked to remove the root, which would leave the tree with
    /// nothing to walk.
    RemoveRoot,
    /// [`Tree::check_sorted`] / [`MutatorTree::check_sorted`] found a
    /// child whose key is below its previous sibling's.
    Unsorted {
        /// The parent whose children are out of order.
        parent: NodeId,
        /// The first child found keyed below its previous sibling.
        child: NodeId,
    },
    /// A [`StructOp::Move`](crate::engine::structure::StructOp::Move)
    /// would put a node under itself or under one of its own
    /// descendants.
    Cycle {
        /// The node being moved.
        node: NodeId,
        /// The parent it was to move under: the node itself or one of
        /// its descendants.
        to: NodeId,
    },
    /// A [`StructOp::Split`](crate::engine::structure::StructOp::Split)
    /// on a node its element cannot cut at the given point
    /// ([`TreeElement::split`] answered `None`), or on the root, which
    /// has no row to put a second part in.
    Unsplittable(NodeId),
    /// A [`StructOp::Merge`](crate::engine::structure::StructOp::Merge)
    /// on a node with no next sibling of an equal key, whose next
    /// sibling has children, or whose element cannot absorb that
    /// sibling ([`TreeElement::merge`] answered `None`).
    Unmergeable(NodeId),
    /// A structural command named a node that an earlier command of the
    /// same walk removed — the node itself, or a node of its subtree.
    TargetRemovedEarlier(NodeId),
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::StaleNode(id) => write!(f, "node {id} is not in the tree"),
            TreeError::PlacementCrossesKey => {
                f.write_str("the placement names a sibling with a different key")
            }
            TreeError::NotAChild { parent, node } => write!(f, "node {node} is not a child of {parent}"),
            TreeError::RemoveRoot => f.write_str("the root of a tree cannot be removed"),
            TreeError::Unsorted { parent, child } => {
                write!(f, "child {child} of {parent} is keyed below its previous sibling")
            }
            TreeError::Cycle { node, to } => {
                write!(
                    f,
                    "node {node} cannot move under {to}, which is itself or in its subtree"
                )
            }
            TreeError::Unsplittable(id) => write!(f, "node {id} cannot be split there"),
            TreeError::Unmergeable(id) => write!(f, "node {id} cannot absorb its next sibling"),
            TreeError::TargetRemovedEarlier(id) => {
                write!(f, "node {id} was removed earlier in the same walk")
            }
        }
    }
}

impl std::error::Error for TreeError {}

/// Arena-backed tree of elements `E`, its children kept sorted by key
/// (see the module docs).
///
/// `cache` is the element type's derived per-tree state; the walker
/// and every mutating method here invalidate it, and the element
/// type's own query methods recompute it on demand.
#[derive(Clone, Debug)]
pub struct Tree<E: TreeElement> {
    /// Backing arena of element nodes. Private to the engine: read it
    /// through [`Tree::arena`]; change it through
    /// [`Tree::insert_child`], [`Tree::update`],
    /// [`Tree::remove_subtree`] or a walk, which keep the
    /// sorted-children invariant.
    pub(in crate::engine) arena: Arena<E>,
    /// Arena id of the tree root.
    pub root: NodeId,
    /// Draw order hint relative to sibling trees in a scene. Higher =
    /// drawn on top.
    pub layer: usize,
    /// The element type's per-tree derived state. Crate-private: it is
    /// derived, so nothing outside the element type's own query
    /// methods may write it. Every way of changing the tree — the
    /// methods here and a walk — marks it stale.
    pub(crate) cache: E::Cache,
}

impl<E: TreeElement> Tree<E> {
    /// A tree whose root is `E::void()`, with a default cache. O(1);
    /// one arena allocation for the root.
    pub fn new_non_indexed() -> Self {
        Self::new_non_indexed_with(E::void())
    }

    /// A tree whose root is `element`, with a default cache. O(1); one
    /// arena allocation for the root.
    pub fn new_non_indexed_with(element: E) -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(element);
        Tree {
            arena,
            root,
            layer: 0,
            cache: E::Cache::default(),
        }
    }

    /// The backing arena, read-only. Navigate it with indextree's
    /// `NodeId` methods (`children`, `descendants`, `parent`, …);
    /// change the tree through [`Self::insert_child`],
    /// [`Self::update`], [`Self::remove_subtree`] or a walk. Besides
    /// the tree, the arena holds every subtree a structural command
    /// detached and a journal still owns: walk from [`Self::root`], or
    /// ask [`Self::contains`], to see only the tree. O(1).
    pub fn arena(&self) -> &Arena<E> {
        &self.arena
    }

    /// Borrow the arena node at `id`, or `None` if it has been
    /// removed. O(1).
    pub fn get(&self, id: NodeId) -> Option<&Node<E>> {
        self.arena.get(id)
    }

    /// Whether `id` names a node **in the tree**: in the arena, and with
    /// [`Self::root`] among its ancestors (or the root itself). A
    /// removed node and a reused slot under an old stamp answer `false`,
    /// and so does a node of a subtree a structural command detached —
    /// still allocated, so a journal can put it back, but not part of
    /// the tree until it does. O(depth of `id`); no allocation.
    pub fn contains(&self, id: NodeId) -> bool {
        self.arena.get(id).is_some() && id.ancestors(&self.arena).last() == Some(self.root)
    }

    /// Pre-order iterator over every descendant of [`Self::root`]
    /// (inclusive). See [`NodeId::descendants`] for iteration order.
    /// No allocation.
    pub fn descendants(&self) -> Descendants<'_, E> {
        self.root.descendants(&self.arena)
    }

    /// Root node id. O(1).
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Direct-child iterator over [`Self::root`]. See
    /// [`NodeId::children`].
    pub fn children(&self) -> Children<'_, E> {
        self.root.children(&self.arena)
    }

    /// Insert `element` as a child of `parent`, in key order: it lands
    /// in the run of `parent`'s children whose key equals its own
    /// [`TreeElement::key`], at the position `at` names within that
    /// run. Returns the new node's id.
    ///
    /// Errors (nothing is inserted): [`TreeError::StaleNode`] for a
    /// `parent` or a placement sibling not in the tree,
    /// [`TreeError::NotAChild`] for a placement sibling under another
    /// parent, [`TreeError::PlacementCrossesKey`] for a placement
    /// sibling with a different key.
    ///
    /// Invalidates the tree's cache. Cost: one arena slot plus a scan
    /// of the siblings between the end of the row and the insertion
    /// point — O(1) for children inserted in ascending key order with
    /// [`Placement::Last`]. The new node is fresh, so no ancestor walk.
    pub fn insert_child(&mut self, parent: NodeId, element: E, at: Placement) -> Result<NodeId, TreeError> {
        let id = siblings::insert_sorted(&mut self.arena, parent, element, at, E::key)?;
        self.cache.invalidate();
        Ok(id)
    }

    /// Remove `node` and its whole subtree, freeing their arena slots;
    /// every id in the subtree becomes stale. This is for building
    /// trees — a removal that can be undone keeps its subtree, and is
    /// the structural command
    /// [`StructOp::Remove`](crate::engine::structure::StructOp::Remove).
    ///
    /// Errors (nothing is removed): [`TreeError::StaleNode`] for a node
    /// not in the tree, [`TreeError::RemoveRoot`] for the root. The
    /// siblings that remain are still in order.
    ///
    /// Invalidates the tree's cache. Cost: O(subtree size).
    pub fn remove_subtree(&mut self, node: NodeId) -> Result<(), TreeError> {
        if self.arena.get(node).is_none() {
            return Err(TreeError::StaleNode(node));
        }
        if node == self.root {
            return Err(TreeError::RemoveRoot);
        }
        node.remove_subtree(&mut self.arena);
        self.cache.invalidate();
        Ok(())
    }

    /// Change the element at `id` with `f` — the one way to mutate an
    /// element outside a walk — and return what `f` returns.
    ///
    /// If `f` changed the element's [key](TreeElement::key), the
    /// element moves to its sorted position among its siblings: after
    /// every sibling keyed at or below its new key, so it ends the run
    /// of that key. It keeps its id and its subtree.
    ///
    /// Errors: [`TreeError::StaleNode`] for an `id` not in the tree
    /// ([`Self::contains`]) — a node of a subtree a structural command
    /// detached included, since that subtree belongs to the journal
    /// that may put it back; `f` is then not called.
    ///
    /// Not journaled: a tree that keeps history changes through
    /// [`Self::update_journaled`].
    ///
    /// Invalidates the tree's cache. Cost: `f`, two O(1) key reads and
    /// an O(depth) check that `id` is in the tree; only when the key
    /// changed, a scan of the siblings the element moves past plus an
    /// O(depth) re-attach.
    pub fn update<R>(&mut self, id: NodeId, f: impl FnOnce(&mut E) -> R) -> Result<R, TreeError> {
        self.update_in(id, None, f)
    }

    /// [`Self::update`], recorded in `journal`: the element's
    /// before-image and position are recorded before `f` runs — unless
    /// `journal` has recorded it already — and a move its key change
    /// causes is noted, so the closed [`Journal`](crate::engine::journal::Journal)
    /// can revert and replay it exactly.
    ///
    /// Errors: [`TreeError::StaleNode`] for an `id` not in the tree; `f`
    /// is then not called. A journaled call is all or nothing for its
    /// whole builder (see [`JournalBuilder`]): on the error, everything
    /// `journal` recorded is reverted and the builder is left empty.
    ///
    /// Cost: [`Self::update`]'s, plus one hash lookup and, on the
    /// element's first record in `journal`, a clone of it.
    pub fn update_journaled<R>(
        &mut self,
        id: NodeId,
        journal: &mut JournalBuilder<E>,
        f: impl FnOnce(&mut E) -> R,
    ) -> Result<R, TreeError> {
        let result = self.update_in(id, Some(&mut *journal), f);
        if result.is_err() {
            journal.unwind(self);
        }
        result
    }

    /// The body of [`Self::update`] and [`Self::update_journaled`].
    fn update_in<R>(
        &mut self,
        id: NodeId,
        mut journal: Option<&mut JournalBuilder<E>>,
        f: impl FnOnce(&mut E) -> R,
    ) -> Result<R, TreeError> {
        if !self.contains(id) {
            return Err(TreeError::StaleNode(id));
        }
        if let Some(journal) = journal.as_deref_mut() {
            journal.record(&self.arena, id);
        }
        let Some(node) = self.arena.get_mut(id) else {
            return Err(TreeError::StaleNode(id));
        };
        let element = node.get_mut();
        let before = element.key();
        let out = f(element);
        let rekeyed = element.key() != before;
        let parent = node.parent();
        if let (true, Some(parent)) = (rekeyed, parent) {
            if let Some(prev) = siblings::reposition_slot(&self.arena, id, E::key) {
                move_among_siblings(&mut self.arena, parent, id, prev, journal);
            }
        }
        self.cache.invalidate();
        Ok(out)
    }

    /// Whether every parent's children are in ascending key order — the
    /// invariant [`Self::insert_child`], [`Self::update`] and the walker
    /// keep. The first out-of-order child, in pre-order, is
    /// [`TreeError::Unsorted`].
    ///
    /// Always compiled, for tests and property checks. Cost: O(nodes)
    /// key reads; no allocation.
    pub fn check_sorted(&self) -> Result<(), TreeError> {
        siblings::check_sorted(&self.arena, self.root, E::key)
    }
}

/// Arena of [`Mutator`]s plus a root id, its children kept sorted by
/// key exactly like a [`Tree`]'s. A separate type from [`Tree`] because
/// mutators carry no derived state — no cache, no layer. Applied to a
/// `Tree<E>` with [`MutatorTree::apply_to`] or
/// [`MutatorTree::apply_at`].
#[derive(Clone, Debug)]
pub struct MutatorTree<E: TreeElement, M: Mutation<E>> {
    /// Backing arena — the same `indextree::Arena` the target `Tree`
    /// uses, so `NodeId`s traverse the mutator tree freely. Private to
    /// the engine for the same reason as [`Tree`]'s: read it through
    /// [`MutatorTree::arena`], add to it through
    /// [`MutatorTree::insert_child`].
    pub(in crate::engine) arena: Arena<Mutator<E, M>>,
    /// Arena id of the mutator root. Every walk starts here.
    pub root: NodeId,
    /// Whether any node is a [`Mutator::Structure`]. Kept by the two
    /// ways a mutator tree gains a node, [`Self::new_with`] and
    /// [`Self::insert_child`], so a walk knows before it starts whether
    /// it will have a structural phase.
    structural: bool,
}

/// Not derived: `derive(Default)` would demand `E: Default` and
/// `M: Default`, which a mutator tree does not need. Available where
/// [`MutatorTree::new`] is: for an element type with a default kind.
impl<E: TreeElement, M: Mutation<E>> Default for MutatorTree<E, M>
where
    E::Kind: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E: TreeElement, M: Mutation<E>> MutatorTree<E, M> {
    /// An empty mutator tree rooted at a [`Mutator::Void`] on channel 0
    /// of the element type's default kind. O(1); one arena allocation
    /// for the root.
    pub fn new() -> Self
    where
        E::Kind: Default,
    {
        Self::new_with(Mutator::new_void(BranchKey::new(E::Kind::default(), 0)))
    }

    /// A mutator tree whose root is `node`. O(1); one arena
    /// allocation.
    pub fn new_with(node: Mutator<E, M>) -> Self {
        let structural = node.is(MutatorType::Structure);
        let mut arena = Arena::default();
        let root = arena.new_node(node);
        MutatorTree {
            arena,
            root,
            structural,
        }
    }

    /// The backing arena, read-only. O(1).
    pub fn arena(&self) -> &Arena<Mutator<E, M>> {
        &self.arena
    }

    /// Borrow the arena node at `id`, or `None` if it has been
    /// removed. O(1).
    pub fn get(&self, id: NodeId) -> Option<&Node<Mutator<E, M>>> {
        self.arena.get(id)
    }

    /// Insert `mutator` as a child of `parent`, in key order — the
    /// same rule and the same errors as [`Tree::insert_child`]. Every
    /// mutator tree is built through this, the builder's included, so
    /// its rows are sorted the way the walker's merge needs.
    ///
    /// Cost: one arena slot plus a scan of the siblings between the
    /// end of the row and the insertion point — O(1) for ascending
    /// [`Placement::Last`] inserts.
    pub fn insert_child(
        &mut self,
        parent: NodeId,
        mutator: Mutator<E, M>,
        at: Placement,
    ) -> Result<NodeId, TreeError> {
        let structural = mutator.is(MutatorType::Structure);
        let id = siblings::insert_sorted(&mut self.arena, parent, mutator, at, Mutator::key)?;
        self.structural |= structural;
        Ok(id)
    }

    /// Whether any node of this mutator tree is a
    /// [`Mutator::Structure`] — whether a walk of it has a structural
    /// phase. O(1).
    pub(in crate::engine) fn is_structural(&self) -> bool {
        self.structural
    }

    /// Whether every parent's children are in ascending key order. See
    /// [`Tree::check_sorted`]. O(nodes) key reads; no allocation.
    pub fn check_sorted(&self) -> Result<(), TreeError> {
        siblings::check_sorted(&self.arena, self.root, Mutator::key)
    }

    /// Whether this mutator tree can change anything below the element
    /// its root lands on: its root is an
    /// [`Instruction`](Mutator::Instruction), or some node below the
    /// root is not a [`Void`](Mutator::Void). A mutator tree that is
    /// its root plus `Void`s changes the element its root lands on and
    /// nothing else.
    ///
    /// Cost: O(mutator nodes) at worst; stops at the first node that
    /// reaches below.
    pub fn reaches_below_root(&self) -> bool {
        if matches!(self.arena[self.root].get(), Mutator::Instruction { .. }) {
            return true;
        }
        self.root
            .descendants(&self.arena)
            .skip(1)
            .any(|id| !matches!(self.arena[id].get(), Mutator::Void { .. }))
    }

    /// Apply this mutator tree anchored at `tree`'s root:
    /// [`Self::apply_at`] with the root as its one anchor and `seed` as
    /// the root's parent's inherited state. The mutator's root applies
    /// to the tree's root when their **kinds** match — the root's
    /// channel is not compared — and when they do not, nothing is
    /// applied anywhere. An element type whose `Inherited` is `()`
    /// passes `&()`.
    ///
    /// The root is always in the tree, so there is no anchor to refuse.
    /// A [structural command](crate::engine::structure) can still fail;
    /// the walk is then undone as a whole — its field changes included —
    /// and the error is logged, since there is nothing to return it to.
    /// A tree that must see the error is changed through
    /// [`Self::apply_at`].
    ///
    /// Invalidates `tree`'s cache once more when the walk ends, on top
    /// of the one each applied mutator node performs, so a walk that
    /// applied nothing still leaves no stale memo behind.
    ///
    /// Cost: O(matching (target, mutator) pairs) — see
    /// [`walk_tree_from`](crate::engine::walker::walk_tree_from) — plus,
    /// for a mutator tree with structural commands, what a journaled
    /// walk costs (see [`Self::apply_at`]).
    pub fn apply_to(&self, tree: &mut Tree<E>, seed: &E::Inherited) {
        let root = tree.root;
        let result = run_walk(tree, self, None, |tree, walk| {
            walk_anchor(tree, self, root, seed, walk)
        });
        if let Err(e) = result {
            error!("engine: a structural command failed, so the walk was undone: {e}");
        }
        tree.cache.invalidate();
    }

    /// Apply this mutator tree at each of `anchors` — typically the
    /// selection — reaching only the anchored subtrees.
    ///
    /// 1. `anchors` resolves to a concrete set of nodes, without
    ///    repeats; [`Anchors`] says how each form resolves.
    /// 2. Only the nodes whose **kind** is the mutator root's stay
    ///    anchors — the root's channel is not compared. A mutator's
    ///    shape is relative to its root, so at a node of another kind it
    ///    does not apply, and nothing below that node is reached.
    /// 3. If this mutator reaches below its root
    ///    ([`Self::reaches_below_root`]), every anchor with another
    ///    anchor among its ancestors is dropped, so a subtree mutation
    ///    reaches each element exactly once. Nesting is decided among
    ///    the anchors of step 2 alone: an anchor is never dropped for an
    ///    ancestor the root would not apply at. A mutator that touches
    ///    only its root keeps every anchor.
    /// 4. `seed_for(tree, anchor)` is called once per remaining anchor,
    ///    **before any anchor is walked**, and returns the inherited
    ///    state of the anchor's **parent**. Every seed therefore
    ///    reflects the tree as it was before the call.
    /// 5. The remaining anchors are walked in tree order: the root
    ///    applies to the anchor element and the walk continues below as
    ///    any walk does. A [`Mutator::Structure`] the walk reaches
    ///    changes nothing yet: its target and its command are collected.
    /// 6. When every anchor has been walked, the
    ///    [structural commands](crate::engine::structure) collected
    ///    are applied in the order the walk reached them.
    ///
    /// All or nothing: every given node is validated — in the tree,
    /// under its root — before anything is resolved or walked, and one
    /// that is not is [`TreeError::StaleNode`], with nothing applied
    /// and no seed taken. A structural command that fails undoes the
    /// whole call — the field changes of step 5 and the commands before
    /// it included — and its error is returned
    /// ([`StructOp`](crate::engine::structure::StructOp) lists them).
    /// Undoing needs a record of what the walk changed, so a mutator
    /// tree with structural commands records into a journal of its own,
    /// which it then discards — freeing every subtree the walk removed,
    /// since a tree changed without history cannot put them back.
    ///
    /// Invalidates `tree`'s cache. Cost, with `g` given nodes, `k`
    /// resolved anchors and `d` the depth they sit at:
    /// - validating is O(g × d) — one walk up to the root per distinct
    ///   given node;
    /// - resolving is O(g) for [`Anchors::Nodes`] and
    ///   [`Anchors::ParentOf`]; [`Anchors::SiblingsOf`] scans the row of
    ///   each distinct parent of the given nodes once, which is what it
    ///   resolves to;
    /// - a single anchor needs nothing more; for `k > 1`, one path
    ///   `Vec` of length `d` per anchor, O(k × d) to drop nested
    ///   anchors, and an O(k log k × d) sort — siblings of different
    ///   keys order by key in O(1), and a row is scanned once, the
    ///   first time two anchors part on equal keys in it;
    /// - then one seed and one walk per kept anchor, O(matching pairs
    ///   below it). Nothing is proportional to the rest of the tree;
    /// - for a mutator tree with structural commands, what
    ///   [`Self::apply_at_journaled`] costs on top, plus each command's
    ///   own cost ([`StructOp`](crate::engine::structure::StructOp)).
    ///
    /// Not journaled: for a tree that needs no history (a glyph layer
    /// that is rebuilt). A tree that keeps history is changed through
    /// [`Self::apply_at_journaled`].
    pub fn apply_at<F>(&self, tree: &mut Tree<E>, anchors: &Anchors, seed_for: F) -> Result<(), TreeError>
    where
        F: Fn(&Tree<E>, NodeId) -> E::Inherited,
    {
        apply_anchored(self, tree, anchors, seed_for, None)
    }

    /// [`Self::apply_at`], recorded in `journal`: the same anchors, the
    /// same nested-anchor rule and the same per-anchor seeds, and every
    /// element the walk applies to is recorded — its before-image and
    /// position, immediately before its first apply within `journal` —
    /// together with every move a key change causes. Close `journal`
    /// into a [`Journal`](crate::engine::journal::Journal) to revert or
    /// replay the walk exactly. An element the walk visits without
    /// applying to it is not recorded. The structural commands are
    /// recorded too: what each inserts, removes or moves, as positions
    /// in the arrangement the builder started from and the one it
    /// closes on (see [`Journal`](crate::engine::journal::Journal)). A
    /// subtree a command removes is detached, not freed: the journal
    /// owns it until [`Journal::discard`](crate::engine::journal::Journal::discard).
    ///
    /// All or nothing for the whole builder: if the call returns an
    /// error, everything `journal` recorded — this call's applies and
    /// those of earlier calls on the same builder — is reverted before
    /// it returns, the builder is left empty, and the error is
    /// returned. The tree is then as it was when the builder was
    /// created, and every node the builder's commands created is
    /// freed. A stale anchor is refused before anything is applied; a
    /// failing structural command, after the walk.
    ///
    /// Cost: [`Self::apply_at`]'s, plus per element applied to one hash
    /// lookup and, on its first record in `journal`, a clone of it; per
    /// moved element O(1) more bookkeeping. Nothing per element visited.
    pub fn apply_at_journaled<F>(
        &self,
        tree: &mut Tree<E>,
        anchors: &Anchors,
        seed_for: F,
        journal: &mut JournalBuilder<E>,
    ) -> Result<(), TreeError>
    where
        F: Fn(&Tree<E>, NodeId) -> E::Inherited,
    {
        let result = apply_anchored(self, tree, anchors, seed_for, Some(&mut *journal));
        if result.is_err() {
            journal.unwind(tree);
        }
        result
    }
}
