// SPDX-License-Identifier: MPL-2.0

//! The two arenas a walk aligns. [`Tree<E>`](crate::engine::tree::Tree) wraps an
//! `indextree::Arena` of elements for O(1) child iteration and O(1)
//! node access, plus the element type's per-tree
//! [`TreeCache`](crate::engine::element::TreeCache);
//! [`MutatorTree<E, M>`](crate::engine::tree::MutatorTree) mirrors that shape but carries
//! [`Mutator`](crate::engine::mutator::Mutator)s instead of elements. The two meet in
//! [`MutatorTree::apply_to`](crate::engine::tree::MutatorTree::apply_to), which hands both arenas to the
//! [walk](crate::engine::walker). Everything here is mutation-first
//! per §B2: changing a field is a mutator applied in place, never a
//! rebuilt arena.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::engine::walker::walk_tree;
use indextree::{Arena, Children, Descendants, Node, NodeId};

/// Arena-backed tree of elements `E`.
///
/// `cache` is the element type's derived per-tree state; the walker
/// invalidates it whenever it applies a mutation, and the element
/// type's own query methods recompute it on demand.
#[derive(Clone, Debug)]
pub struct Tree<E: TreeElement> {
    /// Backing arena of element nodes.
    pub arena: Arena<E>,
    /// Arena id of the tree root.
    pub root: NodeId,
    /// Draw order hint relative to sibling trees in a scene. Higher =
    /// drawn on top.
    pub layer: usize,
    /// The element type's per-tree derived state. Crate-private: it is
    /// derived, so nothing outside the element type's own query
    /// methods may write it, and [`Tree::invalidate_caches`] is the
    /// public way to mark it stale.
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

    /// Borrow the arena node at `id`, or `None` if it has been
    /// removed. O(1).
    pub fn get(&self, id: NodeId) -> Option<&Node<E>> {
        self.arena.get(id)
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

    /// Mark the tree's derived state stale; the next query recomputes
    /// it.
    ///
    /// Call this after any direct arena write that bypasses
    /// [`MutatorTree::apply_to`] (builder-phase writes, for instance).
    /// Mutations applied by the walker invalidate on their own.
    ///
    /// O(1) — [`TreeCache::invalidate`].
    pub fn invalidate_caches(&self) {
        self.cache.invalidate();
    }
}

/// Arena of [`Mutator`]s plus a root id. A separate type from
/// [`Tree`] because mutators carry no derived state — no cache, no
/// layer. Applied to a `Tree<E>` via [`MutatorTree::apply_to`].
#[derive(Clone, Debug)]
pub struct MutatorTree<E: TreeElement, M: Mutation<E>> {
    /// Backing arena — the same `indextree::Arena` the target `Tree`
    /// uses, so `NodeId`s traverse the mutator tree freely.
    pub arena: Arena<Mutator<E, M>>,
    /// Arena id of the mutator root. Every walk starts here.
    pub root: NodeId,
}

/// Not derived: `derive(Default)` would demand `E: Default` and
/// `M: Default`, which a mutator tree does not need.
impl<E: TreeElement, M: Mutation<E>> Default for MutatorTree<E, M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: TreeElement, M: Mutation<E>> MutatorTree<E, M> {
    /// An empty mutator tree rooted at a channel-0
    /// [`Mutator::Void`]. O(1); one arena allocation for the root.
    pub fn new() -> Self {
        Self::new_with(Mutator::new_void(0))
    }

    /// A mutator tree whose root is `node`. O(1); one arena
    /// allocation.
    pub fn new_with(node: Mutator<E, M>) -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(node);
        MutatorTree { arena, root }
    }

    /// Borrow the arena node at `id`, or `None` if it has been
    /// removed. O(1).
    pub fn get(&self, id: NodeId) -> Option<&Node<Mutator<E, M>>> {
        self.arena.get(id)
    }

    /// Walk this mutator tree against `tree`, root against root, and
    /// apply every mutator the walk aligns with a target.
    ///
    /// `seed` is the inherited state of the walk root's **parent**:
    /// the root's mutations receive it, and the walker computes every
    /// deeper element's state from it with
    /// [`TreeElement::inherit`].
    /// An element type whose `Inherited` is `()` passes `&()`.
    ///
    /// Invalidates `tree`'s cache once more when the walk ends, on top
    /// of the one each applied mutator node performs, so a walk
    /// that applied nothing still leaves no stale memo behind.
    ///
    /// Cost: O(matching (target, mutator) pairs) — see
    /// [`walk_tree_from`](crate::engine::walker::walk_tree_from).
    pub fn apply_to(&self, tree: &mut Tree<E>, seed: &E::Inherited) {
        walk_tree(tree, self, seed);
        tree.cache.invalidate();
    }
}
