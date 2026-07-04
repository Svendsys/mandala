# P2-36: Projection-layer hot-path performance bundle — O(samples×nodes) clip filter, per-call arc-length allocations, metric-cache String keys, identity-only mutator signatures, per-frame EdgeKey/String churn

**Severity:** P2 (each item is per-frame or per-event on the mobile budget) · **Area:** baumhard projection + font

Bundle of verified §B1/§B7 findings in the connection/border/portal pipelines. Each is independently fixable; they share benches, so land as a series quoting `./test.sh --bench` deltas.

## Items

1. **Connection clip filter is O(samples × nodes) per frame** (`scene_builder/connection.rs:63-101,422-434`): `point_inside_any_node(p, node_aabbs)` linear-scans every visible node AABB for every sampled glyph of every visible edge, every frame — 300 edges × ~50 samples × 250 nodes ≈ 3.75M AABB tests/frame; plus a fresh `Vec<(f32,f32)>` per edge per frame. **Fix:** per-edge prune — compute the edge AABB from cached samples once, test only intersecting node AABBs (sorted-by-x sweep or uniform grid; `RegionIndexer` is the named seam); reuse a scratch Vec.
2. **Arc-length table allocates per call** (`connection/bezier.rs:56-100`): 257-entry `Vec<f32>` per curved-edge sampling call per drag frame; `cubic_bezier_length` builds the whole table to read `last()`; `distance_to_path` also allocates a dense sample Vec per edge per hit test (`connection/mod.rs:417-440`). **Fix:** stack array `[f32; N+1]` (~1KB); running-sum for length; AABB early-out for `distance_to_path` (control-polygon hull property) — also fixes `hit_test_edge`'s per-click cost (`document/hit_test.rs:199-240`).
3. **Metric cache allocates a String key per lookup — including hits** (`font/metric_cache.rs:69,129-209`): `(face, OrderedFloat(size_pt), grapheme.to_string())` built before the read-lock probe; `border_run_specs` does ≥12 lookups per framed node per rebuild → ~3,000 allocs + lock round-trips/frame at 250 framed nodes. Module-doc cost claims ("~12 bytes/entry", "~100ns") are wrong. **Fix:** two-level map keyed `(face, size_bits)` with inner `FxHashMap<Box<str>, f32>` probed via `&str` (alloc only on miss); correct the doc numbers. Coordinate with P0-06 (same file).
4. **Border/portal mutator trees rebuilt every rebuild on identity-only signatures** (`scene_rebuild.rs:585-705`; `tree_builder/border.rs:146-148`): `border_identity_sequence` hashes only the framed-node id list, so the InPlaceMutator arm re-runs `border_run_specs` + allocates a full MutatorTree (8 deltas/node) even when the user merely clicked an edge label. Section frames already demonstrate the fix (content-covering signature + no-op early-return, `tree_builder/section_frame.rs:50-80`). **Fix:** extend border/portal signatures to hash content; early-return on match.
5. **Per-frame identity churn in the connection pass** (`connection.rs:119-151,376-393`; `scene_cache.rs:185-208,273-283`): 2×`EdgeKey` (6 Strings) per line edge per frame + fresh `seen_keys` HashSet + 2 bucket-key clones per slow-path insert + evicted-keys Vec in `retain_keys`; RenderScene Vecs grow from zero capacity each frame. **Fix:** intern EdgeKey once per edge per frame (index-keyed seen-set), pre-size vecs with `map.edges.len()`, caller-owned scratch.
6. **Label pass rebuilds the connection path it just built** (`label.rs:55-63` vs `connection.rs:347-355`; third build for handles on selected edges). **Fix:** share one `ConnectionPath` per edge per frame (natural fold-in during P1-22 step 3).
7. **Scene cache fast path is config-blind** (`connection.rs:204-227` vs the translate path's guards at :276-292): reuses cached body/font/size with no comparison — correctness rests on every future mutation site remembering `scene_cache.clear()`. **Fix:** apply the same three-comparison guard on the fast path (fall through to slow on mismatch); update the module doc's caller contract.
8. **grapheme_chad copies** (verified): `replace_substring` copies the whole string twice + revalidates UTF-8 on the per-keystroke path — `s.replace_range(i..n, source)` is equivalent (boundaries come from `find_byte_index_of_grapheme`); `split_off_graphemes` collects every cluster into `Vec<&str>` + rebuilds both halves — `find_byte_index_of_grapheme` + `String::split_off` does it with one allocation. Both are benched primitives; quote deltas.

## Acceptance criteria

- Each landed item quotes bench before/after (§B6-style discipline); no correctness regressions (`./test.sh` green).
- Item 7 ships with a stale-config regression test.

## Pointers

CONVENTIONS §B1/§B7; files cited inline; `lib/baumhard/benches/test_bench.rs`.
