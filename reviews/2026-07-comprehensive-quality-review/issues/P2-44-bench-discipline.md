# P2-44: Benchmark-reuse discipline drift — 7 test modules with §T2.2 headers never imported by the bench file, ~156/299 do_* bodies unbenched, one #[test] wrapper calls the wrong body, hot color fns structurally unbenchable

**Severity:** P2 (the §B7/§B8 contract has drifted badly; one real test hole) · **Area:** baumhard tests/benches

## Problems (all verified)

1. **Wrong wrapper — zero coverage for a fundamental**: `test_count_number_of_lines` calls `do_count_grapheme_clusters()` instead of `do_count_number_of_lines()` (`util/tests/grapheme_chad_tests.rs:330-340`) — the 10-case `COUNT_LINES_TEST` table runs only under `cargo bench`, which `./test.sh` never runs. One-line fix; exactly the failure mode §T3's wrapper convention exists to prevent.
2. **Seven §T2.2 modules not imported by test_bench.rs**: `map_children_tests`, `spatial_descend_tests`, `bvh_descent_tests`, `subtree_aabb_tests`, `camera_tests`, `predicate_tests`, `element_tests` — each of whose headers claims "every public body is benchmarkable from benches/test_bench.rs". MapChildren/SpatialDescend are user-visible walker primitives (§B7 "every user-visible primitive has a criterion bench"); camera math runs per pointer event; predicate eval runs inside every RepeatWhile iteration.
3. **Aggregate drift**: set-diff of all `pub fn do_*` (299) vs bench references (143) → ~156 unbenched bodies. No *stale* imports exist (`cargo bench --no-run` compiles clean, verified) — but glob imports mean the compiler can never flag a body that silently loses its entry.
4. **metric_cache primitives** (`glyph_advance`, `glyph_ink`, `cluster_width`) shipped with inline `#[cfg(test)]` tests only — no `do_*`, no bench — despite being the most measurement-sensitive code in scope (§B3/§B7 same-commit rule).
5. **Hot color-conversion fns are structurally unbenchable**: tests for `resolve_var`, `hex_to_rgba_safe`, HSV helpers etc. live in `util/color.rs`'s inline `#[cfg(test)]` module (color.rs:254-817) — the wrong file AND unreachable from the bench harness, even though the test comments themselves say a regression "would be invisible to a smoke test but visible in the frame budget". Move to `util/tests/color_tests.rs` as `do_*`/`test_*` pairs + bench entries.
6. **CONVENTIONS §B6's bench-name list has drifted**: names `region_indexer_insert`, `region_params_calculate_pixel_from_region`, etc. — actual entries are `region_indexer_insert_and_remove`, `region_params_pixel_to_region`, `region_params_region_to_pixel`, `region_rect_exhaustive_4x4_grid`. `zoom_visibility.rs:18,82` cites a `zoom_visibility_contains` bench that doesn't exist.
7. **Pre-convention naming stragglers**: the three oldest suites (model_tests, tree_tests, tree_walker_tests) export bare names (`matrix_place_in_1`, `basics_solo_mutation`) instead of `do_*` + wrapper — mixed convention in the exact surface where renames are compiler-invisible (§B8 two-file rule).

## Fix plan

1. Fix the wrong wrapper (item 1) immediately — it's a live test hole.
2. Add the seven module imports + `bench_function` entries (mechanical; bodies already exist).
3. Move color tests to the tests tree with `do_*` pairs + add `resolve_var`/`hex_to_rgba_safe` benches; add metric_cache `do_*` tests + benches in a new `font/tests/metric_cache_tests.rs`.
4. Correct §B6's names and the zoom_visibility doc references.
5. Rename the pre-convention bodies to `do_*` + wrappers, updating test_bench.rs in the same commit.
6. For the remaining long tail (~150 bodies), either add entries mechanically or amend §B8 with an explicit benched-subset policy — the current text promises something the repo does not do; pick one and make the doc true.

## Acceptance criteria

- `cargo test -p baumhard count_number_of_lines` actually executes the line-count table.
- Every §T2.2 module header's claim is true (imported by the bench file).
- `cargo bench -p baumhard --no-run` compiles; `./test.sh --bench` runs green.
- §B8/§B6 text matches reality.

## Pointers

`lib/baumhard/benches/test_bench.rs`; `lib/baumhard/src/{util,gfx_structs,font,core}/tests/`; CONVENTIONS §B6/§B7/§B8; TEST_CONVENTIONS §T2.2/§T3/§T6.
