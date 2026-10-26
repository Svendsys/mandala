// SPDX-License-Identifier: MPL-2.0

//! The glyph instantiation of the engine's trees: [`GfxTree`](crate::gfx_structs::tree::GfxTree) and
//! [`GfxMutatorTree`](crate::gfx_structs::tree::GfxMutatorTree), the [`GlyphTreeCache`](crate::gfx_structs::tree::GlyphTreeCache) a glyph tree keeps beside
//! its arena, and the spatial queries only glyph trees answer — the
//! AABB memo and the BVH hit-test (`descendant_at`, `descendant_near`,
//! `descendants_aabb`, `ensure_subtree_aabbs`). Mutation goes through
//! [`MutatorTree::apply_to`](crate::engine::tree::MutatorTree::apply_to)
//! as it does for every other element type (§B2); this module only adds what
//! a glyph tree derives.

use crate::engine::element::TreeCache;
use crate::engine::tree::{MutatorTree, Tree};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutation, GlyphTreeEventInstance};
use crate::gfx_structs::util::regions::{RegionIndexer, RegionParams};
use crate::util::geometry::aabb_contains;
use glam::Vec2;
use indextree::{Arena, NodeId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A glyph tree: the engine's [`Tree`] over [`GfxElement`].
pub type GfxTree = Tree<GfxElement>;

/// A glyph mutator tree: the engine's [`MutatorTree`] over
/// [`GfxElement`] and [`GfxMutation`].
pub type GfxMutatorTree = MutatorTree<GfxElement, GfxMutation>;

/// Single-threaded closure that receives a mutable `GfxElement` plus a
/// delivered event. `Rc<RefCell<_>>` because the Mandala event loop is
/// single-threaded (CODE_CONVENTIONS.md §3) and elements are cloned
/// between arenas; the wrapper keeps one callback reachable from every
/// clone without duplicating state. `try_borrow_mut` turns re-entrant
/// delivery into a logged skip rather than a deadlock.
pub type EventSubscriber = Rc<RefCell<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance)>>;

/// A tree node that can receive event deliveries. Implemented by
/// `GfxElement`, which in turn fans events out to its registered
/// [`EventSubscriber`]s.
pub trait TreeEventConsumer {
    /// Handle an event instance. Typically fans out to subscribers
    /// cloned from `self`, so the handler can mutate `self` without
    /// aliasing the subscriber list.
    fn accept_event(&mut self, event: &GlyphTreeEventInstance);
}

/// What a glyph tree derives beside its arena — `GfxElement::Cache`.
///
/// The AABB memo and the subtree-AABB dirty flag are `Cell`s because
/// [`TreeCache::invalidate`] runs through `&self` while the walker
/// holds the arena mutably; the per-node subtree AABBs themselves
/// live on the elements, and this flag says whether they are current.
/// The region index is allocated by the indexed constructor
/// ([`Tree::new`](crate::engine::tree::Tree::new)) but not maintained by the walker today; spatial
/// queries use the BVH descent instead. It is a reserved seam for the
/// named trajectory (see CONVENTIONS.md §B6).
#[derive(Clone, Debug)]
pub struct GlyphTreeCache {
    /// Grid parameters of the region index, when the tree is indexed.
    #[allow(dead_code)]
    region_params: Option<Rc<RegionParams>>,
    /// The region index, when the tree is indexed.
    #[allow(dead_code)]
    region_index: Option<Rc<RegionIndexer>>,
    /// Memoized result of [`Tree::descendants_aabb`](crate::engine::tree::Tree::descendants_aabb).
    /// `None` = not yet computed (or invalidated by a mutation);
    /// `Some(None)` = computed and the tree has no visible areas;
    /// `Some(Some(_))` = computed, valid bbox.
    /// `Cell` (not `RefCell`) is fine because the inner type is
    /// `Copy` and we only ever swap whole values.
    aabb_cache: Cell<Option<Option<(Vec2, Vec2)>>>,
    /// When `true`, per-node `subtree_aabb` caches in the arena are
    /// stale and must be recomputed before the next BVH query.
    /// `true` on construction and after any mutation.
    subtree_aabbs_dirty: Cell<bool>,
}

/// Not derived: a fresh tree's subtree AABBs have never been computed,
/// so the dirty flag starts `true`, where `derive(Default)` would
/// start it `false`.
impl Default for GlyphTreeCache {
    fn default() -> Self {
        GlyphTreeCache {
            region_params: None,
            region_index: None,
            aabb_cache: Cell::new(None),
            subtree_aabbs_dirty: Cell::new(true),
        }
    }
}

impl TreeCache for GlyphTreeCache {
    /// Drop the tree-level AABB memo and mark the per-node subtree
    /// AABBs stale. O(1) — two `Cell` writes; the recomputation is
    /// deferred to the next `ensure_subtree_aabbs` or
    /// `descendants_aabb` call.
    fn invalidate(&self) {
        self.aabb_cache.set(None);
        self.subtree_aabbs_dirty.set(true);
    }
}

impl Tree<GfxElement> {
    /// Indexed tree with a `GfxElement::void()` root. The region index
    /// is allocated but not maintained by the walker today
    /// (CONVENTIONS.md §B6). O(1); one arena allocation for the root.
    pub fn new(region_params: Rc<RegionParams>) -> Self {
        let mut tree = GfxTree::new_non_indexed();
        tree.cache.region_params = Some(region_params);
        tree.cache.region_index = Some(Rc::new(RegionIndexer::default()));
        tree
    }

    /// Find the smallest-AABB `GlyphArea` descendant whose rectangle
    /// contains `point`. Returns its [`NodeId`], or [`None`] if
    /// nothing matches.
    ///
    /// Equivalent to `descendant_near(point, 0.0)`.
    ///
    /// # Algorithm
    ///
    /// Uses a BVH (bounding-volume hierarchy) descent: each node's
    /// cached `subtree_aabb` is checked before recursing into its
    /// children. Subtrees whose aggregate AABB does not contain the
    /// point are pruned entirely.
    ///
    /// # Costs
    ///
    /// O(branching_factor × depth) when subtrees are spatially
    /// disjoint; O(n) worst case when subtree AABBs fully overlap.
    ///
    /// Two `Vec` allocations on the first call after a mutation, one
    /// on every call after that. The AABB computation pass runs only
    /// while its cache is dirty, but `bvh_find` carries its descent
    /// frontier on the heap and so allocates per call — the price of
    /// not recursing over a depth a `.mindmap.json` chooses. The
    /// frontier holds the unprocessed sibling rows along its current
    /// path — one element for a linear chain, O(n) for a shallow
    /// wide tree, O(depth x branching) in general — rather than a
    /// whole subtree.
    ///
    /// When multiple areas contain the point, the smallest by area
    /// wins — the "innermost first" convention.
    pub fn descendant_at(&mut self, point: Vec2) -> Option<NodeId> {
        self.descendant_near(point, 0.0)
    }

    /// Variant of [`Self::descendant_at`] that expands every
    /// `GlyphArea`'s AABB by `slack` pixels on each side before
    /// the containment test. Use for fuzzy hit-tests where the
    /// caller wants to forgive a stylus or fat-finger near-miss
    /// (e.g. edge handles, console close-buttons).
    ///
    /// `slack` must be `>= 0.0`. The shape-refinement step inside
    /// `bvh_find` inflates the shape by `slack` on every side;
    /// a negative value would require a *tightening* of the shape,
    /// which this API does not provide (and which would make the
    /// AABB pre-test and the shape post-test disagree). Debug
    /// builds assert; release builds clamp to `0.0`.
    pub fn descendant_near(&mut self, point: Vec2, slack: f32) -> Option<NodeId> {
        debug_assert!(
            slack >= 0.0,
            "descendant_near: slack must be >= 0.0 (got {slack})"
        );
        let slack = slack.max(0.0);
        self.ensure_subtree_aabbs();
        let mut best: Option<(NodeId, f32)> = None;
        bvh_find(&self.arena, self.root, point, slack, &mut best);
        best.map(|(id, _)| id)
    }

    /// Conservative AABB covering every `GlyphArea` descendant of
    /// [`Self::root`]. Returns `(top_left, bottom_right)` or `None`
    /// if the tree has no visible areas.
    ///
    /// # Costs
    ///
    /// O(n) on the first call after the tree mutates; O(1) on
    /// repeated calls thanks to a per-tree memo. The memo is
    /// cleared by every mutation the walker applies — any other
    /// mutating caller that bypasses the mutator pipeline must call
    /// [`Tree::invalidate_caches`](crate::engine::tree::Tree::invalidate_caches) itself or the bbox will drift.
    pub fn descendants_aabb(&self) -> Option<(Vec2, Vec2)> {
        if let Some(cached) = self.cache.aabb_cache.get() {
            return cached;
        }
        let computed = self.compute_descendants_aabb();
        self.cache.aabb_cache.set(Some(computed));
        computed
    }

    /// Ensure every node's `subtree_aabb` cache is fresh. If the
    /// tree has been mutated since the last computation, performs a
    /// bottom-up (post-order) pass that writes each node's
    /// `subtree_aabb` as the union of its own AABB with its
    /// children's subtree AABBs.
    ///
    /// # Costs
    ///
    /// O(n) on the first call after mutation, plus the one heap
    /// vector `compute_subtree_aabbs` carries its post-order
    /// frontier in. O(1) on subsequent calls while the cache is
    /// clean.
    pub fn ensure_subtree_aabbs(&mut self) {
        if !self.cache.subtree_aabbs_dirty.get() {
            return;
        }
        self.compute_subtree_aabbs();
        self.cache.subtree_aabbs_dirty.set(false);
    }

    /// Bottom-up pass: compute and cache `subtree_aabb` for every
    /// node in the arena. Post-order guarantees that children are
    /// processed before their parents, so each parent can read its
    /// children's already-computed subtree AABBs.
    ///
    /// **Iterative on purpose.** This arena is the projection of a
    /// `.mindmap.json`, whose `parent_id` nesting is untrusted and
    /// effectively unbounded — a linear chain of N nodes is a legal
    /// acyclic tree the loader accepts. Recursing over that depth
    /// overflowed the thread stack and took the process down with
    /// `SIGABRT`, which is not a panic and so has no frame to
    /// degrade (CODE_CONVENTIONS §9). The pass is reached from
    /// [`Self::descendant_at`], so a hostile map used to kill the
    /// editor on the first mouse move across the canvas.
    ///
    /// Cost: O(n), plus one heap vector holding the post-order
    /// frontier — the sum of the unprocessed sibling rows along the
    /// current path — one element for a linear chain, since each
    /// node's only child replaces it, O(n) for a shallow wide tree,
    /// and O(depth x branching) in general.
    /// That vector is a deliberate trade against §B7's
    /// allocation-free preference: the recursive shape did not avoid
    /// the storage, it put the same data on the call stack, where
    /// running out is fatal rather than merely costly. The pass is
    /// gated behind `subtree_aabbs_dirty`, so it runs once per
    /// mutation rather than once per frame.
    fn compute_subtree_aabbs(&mut self) {
        let root = self.root;
        let arena = &mut self.arena;
        // `false` = this node's children still need queueing;
        // `true` = they are done and its own merge may run.
        let mut pending: Vec<(NodeId, bool)> = vec![(root, false)];

        while let Some((node_id, children_done)) = pending.pop() {
            if !children_done {
                pending.push((node_id, true));
                let mut child_opt = arena.get(node_id).and_then(|n| n.first_child());
                while let Some(cid) = child_opt {
                    child_opt = arena.get(cid).and_then(|n| n.next_sibling());
                    pending.push((cid, false));
                }
                continue;
            }

            // This node's own AABB.
            let own_aabb = arena.get(node_id).and_then(|n| {
                let area = n.get().glyph_area()?;
                let bounds = area.render_bounds.to_vec2();
                if bounds.x <= 0.0 || bounds.y <= 0.0 {
                    return None;
                }
                let pos = area.position.to_vec2();
                Some((pos, pos + bounds))
            });

            // Merge the children's subtree AABBs, already computed
            // because post-order got to them first.
            let mut combined = own_aabb;
            let mut child_opt = arena.get(node_id).and_then(|n| n.first_child());
            while let Some(cid) = child_opt {
                if let Some(child_aabb) = arena.get(cid).and_then(|n| n.get().subtree_aabb()) {
                    combined = Some(union_aabb(combined, child_aabb));
                }
                child_opt = arena.get(cid).and_then(|n| n.next_sibling());
            }

            if let Some(node) = arena.get_mut(node_id) {
                node.get_mut().set_subtree_aabb(combined);
            }
        }
    }

    fn compute_descendants_aabb(&self) -> Option<(Vec2, Vec2)> {
        let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut any = false;
        for node_id in self.root.descendants(&self.arena) {
            let Some(node) = self.arena.get(node_id) else {
                continue;
            };
            let Some(area) = node.get().glyph_area() else {
                continue;
            };
            let bounds = area.render_bounds.to_vec2();
            if bounds.x <= 0.0 || bounds.y <= 0.0 {
                continue;
            }
            let pos = area.position.to_vec2();
            any = true;
            if pos.x < min.x {
                min.x = pos.x;
            }
            if pos.y < min.y {
                min.y = pos.y;
            }
            let mx = pos.x + bounds.x;
            let my = pos.y + bounds.y;
            if mx > max.x {
                max.x = mx;
            }
            if my > max.y {
                max.y = my;
            }
        }
        if any {
            Some((min, max))
        } else {
            None
        }
    }
}

/// Return the smallest AABB enclosing both `base` (if `Some`) and
/// `other`. When `base` is `None`, returns `other` unchanged.
///
/// O(1), no allocation. Used by [`Tree::compute_subtree_aabbs`](crate::engine::tree::Tree::compute_subtree_aabbs) to
/// merge a parent's own AABB with its children's subtree AABBs.
fn union_aabb(base: Option<(Vec2, Vec2)>, other: (Vec2, Vec2)) -> (Vec2, Vec2) {
    match base {
        None => other,
        Some((min_a, max_a)) => {
            let (min_b, max_b) = other;
            (
                Vec2::new(min_a.x.min(min_b.x), min_a.y.min(min_b.y)),
                Vec2::new(max_a.x.max(max_b.x), max_a.y.max(max_b.y)),
            )
        }
    }
}

/// The BVH descent behind [`Tree::descendant_near`](crate::engine::tree::Tree::descendant_near): slack-inflated
/// AABB tests with shape refinement, so an ellipse-shaped node doesn't
/// false-hit on its corner AABB. Reads the per-node `subtree_aabb`
/// caches, which the caller has made fresh.
///
/// For each child of `node_id`:
/// 1. Prune: if the child's `subtree_aabb` doesn't contain `point`
///    inflated by `slack`, skip the entire subtree.
/// 2. Test the child's own `GlyphArea` AABB (slack-inflated). On
///    hit, refine via `area.shape.contains_local` so an ellipse
///    hit-tests against its actual shape rather than its bounding
///    rectangle.
/// 3. Descend into the child's children.
///
/// Smallest-area wins on tie (so a smaller element stacked over a
/// bigger one is the hit), and the *first* candidate of an equal
/// area keeps the slot — which is why the descent order below is
/// preserved exactly rather than merely being some depth-first
/// order.
///
/// Cost: O(visited), plus one heap `Vec` holding the pending
/// frontier — the sum of the unprocessed sibling rows along the
/// current path — **one element for a linear chain**, since each
/// node's only child replaces it, O(n) for a shallow wide tree, and
/// O(depth x branching) in general. That allocation is a deliberate trade against §B7:
/// the previous shape did not avoid the storage, it kept the same
/// data on the call stack, where a `.mindmap.json` deep enough to
/// exhaust it aborted the process instead of merely costing memory.
fn bvh_find(
    arena: &Arena<GfxElement>,
    node_id: NodeId,
    point: Vec2,
    slack: f32,
    best: &mut Option<(NodeId, f32)>,
) {
    // Pre-order DFS over an explicit stack rather than the call
    // stack. This arena is the projection of a `.mindmap.json`,
    // whose `parent_id` depth is untrusted and unbounded — a linear
    // chain of N nodes is a legal acyclic tree — and every hit test
    // in the app funnels through here, so recursing over that depth
    // meant a hostile map aborted the process on a mouse move.
    //
    // Children are pushed reversed so the first sibling pops first
    // and a node's own subtree is exhausted before the next
    // sibling: the same order the recursive form visited in, which
    // the tie-break below depends on (equal-area candidates keep
    // the one found first).
    let mut pending: Vec<NodeId> = Vec::new();
    push_children_reversed(arena, node_id, &mut pending);

    while let Some(child_id) = pending.pop() {
        let Some(node) = arena.get(child_id) else {
            continue;
        };
        let element = node.get();

        // 1. Prune: subtree AABB must contain (slack-inflated) point.
        if let Some((st_min, st_max)) = element.subtree_aabb() {
            let slack_vec = Vec2::splat(slack);
            if !aabb_contains(point, st_min - slack_vec, st_max + slack_vec) {
                continue;
            }
        } else {
            continue; // no subtree AABB → no renderable content
        }

        // 2. Check this node's own GlyphArea AABB.
        if let Some(area) = element.glyph_area() {
            let pos = area.position.to_vec2();
            let bounds = area.render_bounds.to_vec2();
            if bounds.x > 0.0 && bounds.y > 0.0 {
                let min = Vec2::new(pos.x - slack, pos.y - slack);
                let max = Vec2::new(pos.x + bounds.x + slack, pos.y + bounds.y + slack);
                if aabb_contains(point, min, max) {
                    let hit = {
                        // Shift into the inflated local frame so
                        // rectangle and ellipse get the same isotropic
                        // fuzzy margin the caller asked for. `slack == 0`
                        // is the exact-hit case (no-op inflation).
                        //
                        // The frame is derived from the very `min` / `max`
                        // the AABB test above used, not recomputed from
                        // `pos` and `bounds`. Float addition is not
                        // associative: for an area at x = 872.0 of width
                        // 22.4, `pos.x + bounds.x` rounds to 894.40002
                        // while `894.40002 - 872.0` rounds to 22.400024,
                        // which is *greater* than `bounds.x`. Refining
                        // against `bounds` therefore rejected points the
                        // AABB test had just accepted — a click exactly on
                        // a rectangle's right or bottom edge fell through
                        // to whatever lay beneath. Whether it bit depended
                        // on the coordinates: the same fixture's y axis
                        // rounds the other way (22.399994), so one edge
                        // could misbehave while the opposite one did not.
                        //
                        // Every hit test in the app funnels through here —
                        // `Tree::descendant_at` backs both the canvas-role
                        // routing and `document::hit_test`'s node and
                        // section picking — so this was a whole-app edge
                        // defect, not a portal one.
                        //
                        // Subtracting the same `min` from both sides makes
                        // the boundary compare exact. For a rectangle the
                        // refinement is then a provable no-op (the
                        // accepted set is exactly the closed AABB); for an
                        // ellipse the frame shifts by at most an ULP and
                        // stays inside the unchanged AABB.
                        let local = point - min;
                        let inflated = max - min;
                        area.shape.contains_local(local, inflated)
                    };
                    if hit {
                        // Tie-break by *original* (un-slacked) area so a
                        // physically smaller element still wins.
                        let size = bounds.x * bounds.y;
                        match *best {
                            Some((_, best_size)) if best_size <= size => {}
                            _ => *best = Some((child_id, size)),
                        }
                    }
                }
            }
        }

        // 3. Descend (the subtree-AABB test above proved at least
        //    one descendant may contain the point).
        push_children_reversed(arena, child_id, &mut pending);
    }
}

/// Append every child of `parent` to `pending` in reverse document
/// order, so a LIFO pop yields them first-to-last.
///
/// Reversing in place over the slice just appended keeps this free
/// of any allocation beyond `pending`'s own growth — the children
/// are pushed once and flipped, never collected into a scratch
/// vector (§B7).
fn push_children_reversed(arena: &Arena<GfxElement>, parent: NodeId, pending: &mut Vec<NodeId>) {
    let start = pending.len();
    let mut child_opt = arena.get(parent).and_then(|n| n.first_child());
    while let Some(child_id) = child_opt {
        child_opt = arena.get(child_id).and_then(|n| n.next_sibling());
        pending.push(child_id);
    }
    pending[start..].reverse();
}
