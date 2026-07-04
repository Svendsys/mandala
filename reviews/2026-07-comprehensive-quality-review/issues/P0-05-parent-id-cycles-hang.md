# P0-05: Loader accepts `parent_id` cycles — first interaction touching the cycle hangs or stack-overflows the app

**Severity:** P0 (DoS on load of hand-edited/hostile file) · **Area:** baumhard/mindmap loader + model walkers · **Found independently by two reviewers**

## Context

`.mindmap.json` is designed to be hand-authored (format/ docs), the console `open` verb loads arbitrary files at runtime, and format/macros.md's threat model states "opening any `.mindmap.json` from an untrusted source IS a privilege event". CODE_CONVENTIONS §9: interactive paths must not panic — and a hang is strictly worse than a panic (the native FreezeWatchdog turns it into an abort after 10s; WASM just freezes the tab).

## Problem

The loader (`lib/baumhard/src/mindmap/loader.rs:43-74`) performs no cycle check on `parent_id`. Three model walkers then run unguarded (`lib/baumhard/src/mindmap/model/mod.rs`):

- `is_hidden_by_fold` (:167-179) walks the parent chain with no visited set — a 2-cycle `a→b→a` with neither folded **never terminates**. Called per node per scene build (`scene_builder/node_pass.rs:130`), so the first frame after load hangs.
- `collect_descendants`/`all_descendants` (:134-148) recurse through `children_of` — the same cycle is infinite mutual recursion → **stack overflow** (abort, uncatchable). Reached from drag (`throttled_interaction/moving_node.rs:117`), delete (`document/topology.rs:289`), custom-mutation scope collection (`document/custom/mod.rs:569`).
- `is_ancestor_or_self` (:151-162) — same shape.

Only `maptool verify` detects cycles (`crates/maptool/src/verify/tree.rs:26-42`) — the app never runs it. `format/validation.md:23` itself admits "A cycle makes `all_descendants` loop forever". maptool's own `export` also recurses unguarded and silently drops cycle members.

## Fix plan

1. **Reject cycles at load** in `load_from_str`: one O(n) pass — for each node, walk `parent_id` with a visited set (or Brent/steps cap at `nodes.len()`), erroring with the offending node id and a message in the loader's existing style ("node '3.1': parent chain contains a cycle (3.1 → 2 → 3.1); fix parent_id"). Same posture as the existing zero-sections rejection.
2. Defense in depth (cheap): cap the three walkers' iterations at `nodes.len()` with a `log::error!` + early-return, so runtime mutations (reparent bugs) can never hang either. Note: the app's reparent path already guards against creating cycles; this protects against loaded state only.
3. Tests: loader rejects `a→b→a` and self-parent `a→a` with the exact message; `maptool verify` and the loader agree.

## Acceptance criteria

- Loading a cyclic map fails fast with a §9-quality error naming the node, on both native and WASM.
- No model walker can loop or overflow on any input the loader accepts.
- `./test.sh` green.

## Pointers

`lib/baumhard/src/mindmap/loader.rs`; `lib/baumhard/src/mindmap/model/mod.rs:122-179`; `crates/maptool/src/verify/tree.rs` (existing cycle detector to mirror); `crates/maptool/src/export.rs:29-61`; format/validation.md.
