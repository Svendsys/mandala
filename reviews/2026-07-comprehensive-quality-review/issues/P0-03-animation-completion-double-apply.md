# P0-03: Animation completion double-applies relative mutations and snapshots undo mid-lerp

**Severity:** P0 (wrong final state + broken undo baseline) · **Area:** mandala/document/animations · **Verified:** yes (tick + completion paths read)

## Context

Timed custom mutations (`AnimationTiming { duration_ms, .. }`) tick per frame via `MindMapDocument::tick_animations` and commit on completion through `apply_custom_mutation` "so the tree animation's commit is indistinguishable from the instant-mode equivalent" (`src/application/document/animations.rs:377-397`).

## Problem

Per advancing frame (`animations.rs:367-374`), the **model** position is lerped toward `to`:

```rust
let lerped = anim.from_node.pos_vec2().lerp(anim.to_node.pos_vec2(), t);
node.position.x = lerped.x as f64;
node.position.y = lerped.y as f64;
```

On the completing frame (`elapsed >= total`, line 355-357 — note: no final lerp to `t=1.0` happens), the instance drains via:

```rust
self.apply_custom_mutation(&anim.cm, &anim.target_id, Some(tree));   // line 387
```

This applies the **full** custom mutation again — on top of the already-lerped state. For any relative mutation (e.g. a position nudge of +50 via `ApplyOperation::Add`), the tree/model already sit at `from + 50·t_prev`; applying the full +50 again lands at `from + 50·(1 + t_prev)` — approaching **double the delta** for animations longer than one frame.

Second defect: the undo snapshot happens **inside** that completion-time `apply_custom_mutation` call, so the "before" state captured is the mid-lerp position, not `from`. Ctrl-Z restores `from + 50·t_prev`.

Third (latent): the `None`-tree fallback (`animations.rs:388-395`) writes `node.position = to_node.position` with **no undo entry at all** ("caller's responsibility" — no caller takes it).

## Why no test caught it

Every animation test passes `tree = None` (`tests_mutations.rs:1242-1458`, `tests_hit_move.rs:319-512`), exercising only the fallback. The production `Some(tree)` completion path has zero coverage.

## Fix plan

1. At completion, **reset the target to `anim.from_node` state** (model + tree) before routing through `apply_custom_mutation`, so the full mutation applies exactly once from the true baseline — and the undo snapshot inside it captures `from`.
   - Alternative: apply the mutation to a `from`-based scratch state and assign; or make completion set state to a precomputed `to` and push a hand-built undo entry from `from_node`. The reset-then-apply approach keeps the "indistinguishable from instant-mode" property.
2. Push a `MoveNodes` (or equivalent) undo entry in the `None`-tree fallback so no completion path is un-undoable.
3. Tests (same commit): completion with a live tree after ≥2 ticks — assert final position exactly `from + delta` (not more), and Ctrl-Z restores exactly `from`. Cover both a relative (Add) and an absolute (Assign) mutation.

## Acceptance criteria

- Animated relative mutations end at exactly the same state as their instant-mode equivalents.
- Undo after an animated mutation restores the exact pre-animation state.
- `./test.sh` green; new tests fail on current main.

## Pointers

`src/application/document/animations.rs:343-401` (tick_animations); `src/application/document/custom/mod.rs:179-217` (snapshot site); `src/application/app/drain_frame.rs:121-145` (drain_animation_tick + rebuild); TEST_CONVENTIONS §T1.
