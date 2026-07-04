# P1-23: O(N²) model walks on hot paths — `children_of` full-map scan per call, `all_descendants` per drag frame, `is_hidden_by_fold` re-walked per element per pass

**Severity:** P1 (quadratic scaling on the mobile-budget hot path) · **Area:** baumhard/mindmap model + all consumers · **Found independently by two reviewers**

## Problem

`MindMap::children_of` (`lib/baumhard/src/mindmap/model/mod.rs:122-130`) filters **all** `nodes.values()` and sorts, per call. Consumers:

- `build_children_recursive` calls it once per visited node → **O(N²) per `build_tree()`** (252 full scans on the testament fixture) — and `build_tree` runs on every structural rebuild: text-edit commit, selection change (`rebuild_all` in click paths), undo, animation tick, even a cursor-move path (`event_cursor_moved.rs:779`).
- `all_descendants` recurses through `children_of` → O(N²); `MovingNodeInteraction::drain` calls it **per dragged node per drained frame** (`throttled_interaction/moving_node.rs:117`), cloning every descendant id into the offsets map each frame. A subtree drag on a few-thousand-node map does millions of string compares per frame before any rendering.
- `is_hidden_by_fold` is an O(depth) parent-chain hash-walk re-run per node in node_pass + border_node_data and per edge-endpoint in the connection, label, and portal passes — O((N+2E)·depth) per scene build, ~4× per frame during drags. CONCEPTS §3 claims it "runs once per scene build" (false). Inside `build_children_recursive` the check is redundant by construction (only the direct parent's folded bit can differ at that point).
- maptool's `export.rs:29-61` already hand-rolled a one-pass `ChildIndex` to dodge `children_of` — a §1 "second implementation" symptom proving the need.

## Fix plan

1. **Child index in the model API**: `MindMap::child_index() -> HashMap<Option<&str>, Vec<&MindNode>>` (one O(N) pass, children pre-sorted by `id_sort_key`). Use it in: `build_mindmap_tree` recursion, `all_descendants`/`collect_descendants`, custom-mutation scope collection, tree_cascade; replace maptool's private copy.
2. **Snapshot the descendant set once at drag start** (it cannot change mid-drag); `MovingNodeInteraction` holds it instead of recomputing per drain.
3. **Fold-visibility memo**: compute the hidden-set once per build (O(N): hidden iff parent hidden or parent folded) and thread `&HashSet<&str>` (or a closure) through scene passes; in tree recursion pass a `parent_folded` bool down. Fix the CONCEPTS cost sentence.
4. Keep `children_of` for one-shot callers, adding the §B9 cost note it's missing ("O(n) scan + sort per call — do not call in loops; use `child_index()`").
5. Add cost documentation to `all_descendants`/`is_ancestor_or_self` too (the struct doc promises per-method costs; they have none).

## Acceptance criteria

- `build_tree()` is O(N log N) or better (index build + sorted grouping).
- No per-frame `all_descendants` calls during node drags (verified by reading the drain path).
- `./test.sh` green; scene/tree-build benches quoted before/after (§B1 — this is the mobile budget).

## Pointers

`lib/baumhard/src/mindmap/model/mod.rs:115-192`; `lib/baumhard/src/mindmap/tree_builder/node.rs:300-336`; `src/application/app/throttled_interaction/moving_node.rs:114-152`; `lib/baumhard/src/mindmap/scene_builder/*` (fold checks); `crates/maptool/src/export.rs:29-61`; CONVENTIONS §B1/§B7; CONCEPTS §3 (Fold state).
