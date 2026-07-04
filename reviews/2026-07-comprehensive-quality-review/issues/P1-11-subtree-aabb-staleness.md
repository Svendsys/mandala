# P1-11: Subtree-AABB dirty flag goes stale when a walk mixes SpatialDescend with later mutations — hit tests read stale geometry until the next apply

**Severity:** P1 (hit-test correctness, silent) · **Area:** baumhard/gfx_structs

## Problem

`MutatorTree::apply_to` (`lib/baumhard/src/gfx_structs/tree.rs:97-106`) invalidates the AABB caches **before** the walk:

```rust
target.aabb_cache.set(None);
target.subtree_aabbs_dirty.set(true);
walk_tree_from(target, &self, target.root, self.root)
```

`Instruction::SpatialDescend` calls `gfx_tree.ensure_subtree_aabbs()` **mid-walk** (`tree_walker.rs:576-584`), which recomputes and clears the dirty flag. Element-level mutations applied later in the *same* walk cannot re-set the tree-level `Cell` (they only see `&mut GfxElement`). Consequences:

1. A second `SpatialDescend` later in the same mutator tree resolves against AABBs stale relative to intervening position mutations.
2. After `apply_to` returns, the flag remains `false` — subsequent `descendant_at` / `descendant_near` / `Scene::component_at` hit-tests use subtree AABBs that don't reflect mutations applied after the mid-walk `ensure`, until the next `apply_to` re-dirties. Silent wrong-hit.

Exposure today is via JSON custom mutations combining SpatialDescend with moves (the DSL explicitly supports the shape); the failure is invisible in tests because no test mixes them.

## Fix plan

1. Move the invalidation to **after** the walk in the `Applicable` impl:

```rust
walk_tree_from(target, &self, target.root, self.root);
target.invalidate_caches();
```

(Keeping a pre-walk invalidate too is harmless; the post-walk one is the correctness fix.)

2. Regression test: one mutator tree that (a) SpatialDescends (forcing the mid-walk ensure), then (b) moves a node; assert a post-apply `descendant_at` at the node's *new* position hits it and at the *old* position misses.

## Acceptance criteria

- New test fails on current main, passes after.
- Existing BVH/subtree-AABB tests unchanged; `./test.sh --bench` shows no walk regression (the change is two Cell writes per apply).

## Pointers

`lib/baumhard/src/gfx_structs/tree.rs:97-106, 354-376`; `lib/baumhard/src/gfx_structs/tree_walker.rs:570-603`; TEST_CONVENTIONS §T1 (geometry/hit-test is a fundamental).
