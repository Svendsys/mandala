# Baumhard font/util/format review — findings

## Architecture assessment

The reviewed surface (lib/baumhard/src/font, util, format, build.rs, benches, bin) is in strikingly good shape for its idioms: doc comments with explicit cost notes are the norm rather than the exception, the cosmic-text import boundary holds perfectly across the whole workspace (zero violations), the grapheme/attrs bridges are genuinely Unicode-correct and tested against ZWJ/regional-indicator inputs, and the app crate does not re-implement any color math. The weaknesses are of a different kind: the foundation has accumulated a shadow layer of dead or half-real machinery (an entire dead palettes module, dead macros, a `glyph_ink_height` that does full shaping work to return a constant and has zero callers), the `metric_cache` module quietly broke the crate's own lock discipline (internal `FONT_SYSTEM.write()` with no timeout, reachable while the renderer holds the guard — a latent single-threaded self-deadlock), the benchmark-reuse contract has drifted badly (~156 of 299 `do_*` bodies are unbenched, and the perf-critical color-conversion functions are structurally unbenchable because their tests sit in the wrong module under `#[cfg(test)]`), and the build-time font scan generates the `AppFont` enum in nondeterministic order and won't notice newly dropped-in fonts. Three primitive-level bugs were confirmed empirically by running probes against the built crate (`delete_front_unicode(s, 0)` eats a grapheme; the grapheme-indexed line finder cannot see CRLF line breaks and disagrees with its byte-indexed sibling; `replace_graphemes_until_newline`'s doc/return contract is wrong for multi-line sources).

---

### 1. `delete_front_unicode(s, 0)` deletes one grapheme instead of none
Severity: P0 | Category: correctness | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:399-416; callers: /home/user/mandala/lib/baumhard/src/gfx_structs/model/component.rs:178 (`discard_front`), /home/user/mandala/lib/baumhard/src/gfx_structs/model/line.rs:257,291,320, /home/user/mandala/src/application/app/console_input/edit.rs:164
Evidence: Empirically confirmed by running against the built crate:
```
delete_front_unicode("abcd", 0) => "bcd"   (expected "abcd")
delete_back_unicode("abcd", 0)  => "abcd"  (correct)
```
The loop body increments `grapheme_count` and adds the grapheme's byte length **before** testing `if grapheme_count >= n { break; }` — with `n == 0` the first iteration passes `1 >= 0`, breaks with `char_count = len(first grapheme)`, and `s.drain(0..char_count)` removes it. `delete_back_unicode` checks `grapheme_count > n` *before* accumulating, so it handles `n == 0` correctly — the pair is asymmetric.
Why it matters: §B0 ("the foundation must be pristine... Unicode-correct"), §T1 (grapheme handling is a fundamental). Reachability: the console's `kill_to_start` guards `cursor == 0` before calling (edit.rs:161-163) — a workaround-shaped guard. But `GlyphLine`'s delete-range splitting calls `comp.discard_front(end - begin)` / `discard_front(end - e_begin_comp)` (line.rs:257, 291) where boundary-aligned ranges naturally produce `0`; any such call silently eats a rendered glyph. `REMOVE_PREFIX_TESTS` has no `n = 0` case, which is why this survived.
Fix: Early-return `if n == 0 { return; }` (or restructure the loop to test before accumulating, mirroring `delete_back_unicode`); add `("abcd", 0, "abcd")` to `REMOVE_PREFIX_TESTS` and a matching `n = 0` case to `TRUNCATE_TESTS` in the same commit.
Effort: S

### 2. `metric_cache` acquires `FONT_SYSTEM.write()` internally — latent same-thread self-deadlock under the renderer's guard, bypassing the crate's own lock helper
Severity: P1 | Category: correctness | Confidence: high (structural violation) / med (live trigger today)
Files: /home/user/mandala/lib/baumhard/src/font/metric_cache.rs:214-216,252-254,282-284; /home/user/mandala/src/application/renderer/scene_buffers.rs:34-42; /home/user/mandala/lib/baumhard/src/mindmap/border.rs:501-538,751-791,870; /home/user/mandala/lib/baumhard/src/font/fonts.rs:306-332
Evidence: `shape_advance` / `shape_ink_height` / `shape_ink_extent` all do `FONT_SYSTEM.write().expect(...)` — a plain blocking acquire with **no timeout**. Meanwhile the renderer holds the write guard across the whole border rebuild loop and calls into the cache from inside it:
```rust
// scene_buffers.rs:34-38
let mut font_system = fonts::acquire_font_system_write("rebuild_border_buffers");
for elem in border_elements {
    let specs = baumhard::mindmap::border::border_run_specs(...)  // -> glyph_ink()/glyph_advance()
```
On a cache **miss** inside that loop, `glyph_ink` write-acquires the lock the same thread already holds: on Linux's futex `RwLock` that is a permanent hang; on WASM a panic. fonts.rs:309 states "Every `FONT_SYSTEM.write()` call site in the codebase should go through this helper" precisely so a re-entrant acquire produces a diagnostic panic instead of a hang — metric_cache bypasses it in all three places. It doesn't fire today only because the tree-builder path (`tree_builder/border.rs:237,279`, reached from `document/mod.rs:490` outside any guard) usually warms the same keys first — an ordering accident, not a contract; the flat `BorderElement` pipeline and the tree pipeline are independent, so a style/size/grapheme first seen by the flat path measures cold under the guard.
Why it matters: §B5 lock discipline; §9 (an interactive-path hang is the one failure this codebase cannot tolerate). The crate already solved this shape: `measure_glyph_ink_bounds` / `measure_text_block_unbounded` deliberately take `&mut FontSystem` "so the primitive composes with existing call sites that already hold the write guard" (fonts.rs doc). metric_cache abandoned that design.
Fix: Add `_with(font_system: &mut FontSystem, ...)` variants to the cache API and thread `&mut FontSystem` through `border_run_specs`; guard-holding callers pass their guard, unlocked callers use a thin wrapper. At minimum, replace the three raw `.write()` calls with `acquire_font_system_write("metric_cache::...")` so a future re-entrant miss dies loudly with a site name instead of freezing the app.
Effort: M

### 3. CRLF: `find_nth_line_grapheme_range` cannot see `\r\n` line breaks — disagrees with `count_number_lines` and `find_nth_line_byte_range`
Severity: P1 | Category: correctness | Confidence: high (divergence confirmed) / med (CRLF reachability in buffers)
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:155 (`graph == "\n"`), :140-169, :176-202, :129-131, :18-22
Evidence: Empirically confirmed:
```
count_number_lines("a\r\nb")              = 2
find_nth_line_grapheme_range("a\r\nb", 0) = Some((0, 3))   // swallows the break
find_nth_line_grapheme_range("a\r\nb", 1) = None           // line 1 "does not exist"
find_nth_line_byte_range("a\r\nb", 1)     = Some((3, 4))   // line 1 exists
```
Under UAX #29 `"\r\n"` is one grapheme cluster, so the grapheme walk's `graph == "\n"` never matches; the byte-level siblings split on the raw `\n`. Three primitives that must agree return incompatible line models for the same string. `slice_to_newline` / `replace_graphemes_until_newline` additionally split the `\r\n` cluster mid-grapheme (the `\r` stays inside the line tail and is counted/overwritten as a standalone grapheme). Reachability: Windows-origin paste or any loaded text containing CRLF; nothing normalizes it away.
Why it matters: §B3 "Unicode correctness is a load-bearing invariant"; §T1 "Test the surprising inputs before the obvious ones" — the 580-line test file contains no `\r\n` case.
Fix: Pick the contract once: (a) treat `"\r\n"` (and lone `"\r"`) as terminators in the grapheme walk so ranges align with the byte variant, or (b) mandate LF-only buffers and normalize at every input boundary. Add CRLF rows to `NTH_LINE_*_TEST`, `COUNT_LINES_TEST`, `SLICE_TO_NEWLINE_TEST` in the same commit.
Effort: M

### 4. `glyph_ink_height` is a constant function in disguise, has zero callers, and its docs describe behavior it does not have
Severity: P1 | Category: dead-code | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/font/metric_cache.rs:154-167, :281-328 (`shape_ink_height`), :50-56 (module-doc claim), :103-104 (`INK_HEIGHT_CACHE`)
Evidence: `shape_ink_height` allocates a buffer, runs `set_text` + `shape_until_scroll` under a blocking `FONT_SYSTEM.write()` — then:
```rust
for _run in buffer.layout_runs() {
    return size_pt;
}
0.0
```
and the caller maps `0.0` back to `size_pt` (`if measured > 0.0 { measured } else { size_pt }`), so **every** input returns exactly `size_pt` after paying full shaping cost on first call. The 25-line inline comment concedes "we return `size_pt` for any non-degenerate glyph... Future refinement: use swash bounds". The module doc (:52-56) still advertises it as what prevents "visible gaps" on vertical rails — the real vertical-rail code uses `glyph_ink(...).ink_height` (border.rs:533-538), which measures properly. Workspace-wide grep: **no callers** outside its own unit test.
Why it matters: CLAUDE.md §5 ("NEVER defer the hard parts... ship a 'good enough' now") is violated verbatim by the inline comment; §B9 ("a doc comment that lies is worse than no doc comment"); §5 no dead code. Also burns a `FONT_SYSTEM` write acquisition per unique key for nothing.
Fix: Delete `glyph_ink_height`, `shape_ink_height`, `INK_HEIGHT_CACHE`, and their unit test; rewrite the module-doc "Public API" section around `glyph_advance` / `glyph_ink` / `cluster_width` (the surfaces that exist and are used).
Effort: S
### 5. Color logic split across five files with a dead divergent macro and a duplicated cosmic bridge — single-source-of-truth map and unification proposal
Severity: P1 | Category: ssot | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/color.rs (817 lines), /home/user/mandala/lib/baumhard/src/util/color_conversion.rs (251), /home/user/mandala/lib/baumhard/src/font/color.rs (32), /home/user/mandala/lib/baumhard/src/font/hex.rs (30), /home/user/mandala/lib/baumhard/src/util/palettes.rs (108), /home/user/mandala/lib/baumhard/src/font/attrs.rs:276-279
Evidence — what each file actually does:
- `util/color.rs` — types (`FloatRgba`, `Rgba`, `Palette`, byte-packed `Color` with wrapping Add/Sub/Mul/Div), channel-index consts, macros `rgba!`/`rgb!`/`hex!`, plus `pub use super::color_conversion::*` (:13) so every conversion item has two import paths. Of its 817 lines only ~253 are code; :254-817 is an inline `#[cfg(test)]` module.
- `util/color_conversion.rs` — the real conversion SSOT: `convert_f32_to_u8`/`convert_u8_to_f32`, `resolve_var`, `hex_to_rgba(_safe)`, `hsv_to_rgb`/`rgb_to_hsv`/`hsv_to_hex`/`hex_to_hsv_safe`, `rgba_to_hex`, `hex_with_alpha_scaled`, `from_hex`, `add_rgba`.
- `font/color.rs` — cosmic bridge (`cosmic_color_from_rgba`/`_to_rgba`), correctly delegating to `convert_*`. Declares itself the "single source of ... quantisation at the cosmic-text wall".
- `font/hex.rs` — `hex_to_cosmic_color`, correctly composing `hex_to_rgba` + `cosmic_color_from_rgba`.
- `util/palettes.rs` — dead (see finding 9).
Overlaps found:
(a) `font/attrs.rs:276-279` private `rgba_to_color` is a byte-for-byte re-implementation of `font/color.rs::cosmic_color_from_rgba` — the module that declares itself the single cosmic-color bridge is bypassed by its sibling in the same directory.
(b) The `hex!` macro (util/color.rs:52-72) is a second, divergent hex parser: no 3/4-digit shorthand, `unwrap_or(0)` per pair instead of failing, and `&color[i..i + 2]` **panics on odd-length input** (`hex!("#abc")` → byte-slice out of range), directly contradicting its own doc ("Tolerates a leading `#` and falls back to 0.0 per channel"). It has zero users outside its own tests (workspace grep).
(c) Tests for `color_conversion` functions (`resolve_var`, `hex_to_rgba_safe`, `hex_with_alpha_scaled`, all HSV helpers, `rgba_to_hex`) live in `util/color.rs`'s inline test module — the wrong file, and (because inline `#[cfg(test)]`) unreachable from the bench harness even though the test comments themselves say "any regression from O(1) HashMap lookup to linear scan here would be invisible to a smoke test but visible in the frame budget" (color.rs:405-413). The functions the codebase says are scene-build-hot are the only color functions that *cannot* be benchmarked (§B7, §T2.2).
(d) App crate: clean — no re-implementation found (`from_str_radix` appears nowhere in src/ for colors; `ColorValue::parse` at src/application/console/traits/color_value.rs:36-44 duplicates only the *shape check* `matches!(rest.len(), 3|4|6|8) && all(is_ascii_hexdigit)`, which can drift from `hex_to_rgba`'s acceptance rules).
Why it matters: §5 "Avoid duplicating logic"; §6 module boundaries as promises — `font/color.rs`'s stated promise is broken by (a); a dead panicking macro violates §B0's pristine-foundation bar.
Fix (concrete unification):
1. Delete `hex!` (dead; `from_hex`/`hex_to_rgba` cover every use) and `rgb!` (also zero non-test users) — or, if the compile-time-literal seam is worth keeping for palette authors, keep only `rgba!` (the one with a real user) and re-express `rgb!` via it.
2. Replace `attrs.rs::rgba_to_color` with `crate::font::color::cosmic_color_from_rgba`.
3. Move the color.rs:254-817 inline tests into `util/tests/color_tests.rs` as `do_*`/`test_*` pairs (the file already exists and follows §T2.2), and add bench entries for `resolve_var` + `hex_to_rgba_safe` — closing (c) and finding 6 simultaneously.
4. Have `ColorValue::parse` validate via `baumhard::util::color::hex_to_rgba(t).is_some()` instead of its own shape check.
5. Keep the `color.rs` / `color_conversion.rs` file split (types vs conversions is a fine §6 boundary) but drop the blanket `pub use super::color_conversion::*` re-export in favor of explicit re-exports, so each item has one canonical path.
Effort: M

### 6. Benchmark-reuse contract has drifted: ~156 of 299 `do_*` bodies are unbenched; 7 in-scope bodies missing; metric_cache primitives have neither `do_*` nor bench
Severity: P1 | Category: testing | Confidence: high
Files: /home/user/mandala/lib/baumhard/benches/test_bench.rs; /home/user/mandala/lib/baumhard/src/font/tests/attrs_tests.rs:155,434,470; /home/user/mandala/lib/baumhard/src/font/tests/fonts_tests.rs:362; /home/user/mandala/lib/baumhard/src/util/tests/geometry_tests.rs:115; /home/user/mandala/lib/baumhard/src/util/tests/ordered_vec2_tests.rs:16,31; /home/user/mandala/lib/baumhard/src/font/metric_cache.rs:330-410
Evidence: Set-diff of all `pub fn do_*` in src (299) vs `do_*` referenced in test_bench.rs (143, of which 4 are bench-local helpers): ~156 bodies have no bench entry. In-scope missing entries: `do_attrs_list_slice_at_zwj_grapheme_boundary`, `do_rich_text_spans_slice_at_grapheme_boundary`, `do_rich_text_spans_slice_at_zwj_grapheme_boundary`, `do_family_name_of_round_trips`, `do_option_almost_equal`, `do_ordered_vec2_round_trips_through_hashmap`, `do_ordered_vec2_distinguishes_close_floats_in_hashset`. Out-of-scope clusters (listed for completeness per the audit): all `do_camera_mutation_*`, `do_comparator_*`, `do_predicate_*`, `do_descendant_at_*`/`do_descendant_near_*` (beyond the 3 benched), `do_subtree_aabb_*` (17), `do_region_indexer_*` details (20+), `do_rect_*` (20+), `do_spatial_descend_*`, `do_zip_*`. All *imports* in the bench file resolve — `cargo bench -p baumhard --no-run` compiles with zero warnings, exit 0 (verified; took 2m44s) — but glob imports (`use ...::tests::x::*`) mean the compiler can never flag a body that silently loses its entry. New metric_cache primitives (`glyph_advance`, `glyph_ink`, `cluster_width`) shipped with inline `#[cfg(test)]` tests only — no `do_*`, no bench — despite §B3/§B7 requiring same-commit bench for user-visible primitives, and despite being the most measurement-sensitive code in scope.
Also: CONVENTIONS.md §B6's authoritative bench-name list has drifted from reality — it names `region_indexer_insert`, `region_params_calculate_pixel_from_region`, `region_params_calculate_region_from_pixel`, `region_params_calculate_regions_intersected_by_rectangle`; the actual entries are `region_indexer_insert_and_remove`, `region_params_pixel_to_region`, `region_params_region_to_pixel`, `region_rect_exhaustive_4x4_grid` (test_bench.rs:264-281).
Why it matters: §B7 "Every user-visible primitive has a criterion bench"; §B8/§T6 two-file discipline "is the only thing that keeps it accurate" — it hasn't.
Fix: Add the 7 in-scope missing entries; convert metric_cache tests to `do_*`/`test_*` in a new `font/tests/metric_cache_tests.rs` (matching the sibling files) with bench entries; update §B6's name list. For the ~150 out-of-scope stragglers, either add entries mechanically or amend §B8 to name an explicit benched-subset policy — the current text promises something the repo does not do.
Effort: L

### 7. `test_count_number_of_lines` calls the wrong body — `count_number_lines` has zero test coverage under `cargo test`
Severity: P1 | Category: testing | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/tests/grapheme_chad_tests.rs:330-340; /home/user/mandala/lib/baumhard/benches/test_bench.rs:303
Evidence:
```rust
#[test]
pub fn test_count_number_of_lines() {
    do_count_grapheme_clusters();          // <-- copy-paste: wrong do_* body
}
pub fn do_count_number_of_lines() { ... } // never called by any #[test]
```
The only executor of `do_count_number_of_lines`'s assertions is the criterion bench (test_bench.rs:303), which nobody runs on every commit (`./test.sh` doesn't). The 10-case `COUNT_LINES_TEST` table is dead weight in the default suite.
Why it matters: §T1 lists this family as a fundamental; a silent wrapper mismatch is exactly the failure mode §T3's wrapper convention is supposed to make impossible.
Fix: One-line change: call `do_count_number_of_lines()`.
Effort: S

### 8. build.rs: `AppFont` enum order is nondeterministic build-to-build, and adding a font file does not trigger a rebuild
Severity: P1 | Category: correctness | Confidence: high
Files: /home/user/mandala/lib/baumhard/build.rs:74,118-134 (HashMap + into_iter), :131 (rerun-if-changed per file only); /home/user/mandala/CONCEPTS.md ("drop a font file in, recompile, and the variant appears")
Evidence: `collect_fonts` accumulates into `std::collections::HashMap` and emits `fonts_map.into_iter()` order — randomized per process. Verified empirically: two generated `generated_fonts_data.rs` files in target/ from this machine list `AliceInWonderland, DistorRegular, Flowers3, ...` vs `NotoSansHebrewRegular, RazzleDazzle, ...` — completely different variant orders. Consequences: non-reproducible builds; `AppFont` discriminant/order differs per binary (serde uses variant names so save files survive, but any future ordinal use, FONT_DATA layout, and generated-file diffs churn arbitrarily). Separately, the script prints `cargo:rerun-if-changed` **only for each font file found** (:131) — once any rerun-if-changed is emitted, cargo tracks only those paths, so dropping a *new* .ttf into `src/font/fonts/` does not rerun the script and the variant silently does not appear, contradicting CONCEPTS.md's stated workflow. Minor same-file issues: dedup key is the lowercased filename prefix before `-`, so same-extension collisions would pick a walk-order-dependent winner (today's four collisions — treeroot, pictopeople, appletea, arigatou — are all otf+ttf pairs, which the ttf-preference rule resolves deterministically); extension checks are case-sensitive (`Foo.TTF` is skipped).
Why it matters: deterministic codegen is table stakes for a foundation crate (§B0); the silent no-rebuild directly breaks the documented font workflow.
Fix: `fonts.sort_by(|a, b| a.0.cmp(&b.0))` before generating (one line closes the determinism gap); add `println!("cargo:rerun-if-changed={FONT_DIR}")` for the directory tree (walkdir the dirs and emit per-directory lines so file additions/removals invalidate); compare extensions case-insensitively; on dedup collision with equal preference, pick lexicographically-smallest path instead of walk order.
Effort: S
### 9. Dead-code cluster: palettes module, hex!/rgb! macros, get_font_source/get_some_font, _template bench, empty test module
Severity: P2 | Category: dead-code | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/palettes.rs (whole file); /home/user/mandala/lib/baumhard/src/util/color.rs:38-43 (`rgb!`), :52-72 (`hex!`); /home/user/mandala/lib/baumhard/src/font/fonts.rs:381-392 (`get_font_source`, `get_some_font`); /home/user/mandala/lib/baumhard/benches/_template.rs; /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:525-526
Evidence: Workspace-wide greps found zero references to `TOTAL_WHITE`, `TOTAL_BLACK`, `LIGHT_FOREST`, `SMOOTH_OCEAN`, or any of the 13 `hex_palettes` lazy statics (util/palettes.rs is 108 lines of pure dead weight; its own module doc says it exists "to keep the colour-math tests and demo paths from open-coding magic numbers", but no test or demo uses it). `rgb!` and `hex!` have no users outside color_tests.rs. `get_font_source` and `get_some_font` (fonts.rs) have zero callers anywhere — and `get_some_font` is the only production-code consumer of the `rand` dependency in baumhard's lib (the stress-map *bin* legitimately uses rand), carries a bare `.unwrap()`, and is doc-marked "Test-only helper" while living un-cfg'd in production code. `benches/_template.rs` has no `[[bench]]` entry yet is auto-discovered and compiled as its own bench executable on every `cargo bench` (verified in the `--no-run` output: "Executable benches/_template.rs (target/release/deps/_template-...)"). `grapheme_chad.rs` ends with an empty `#[cfg(test)] mod test {}`.
Why it matters: §5 "no dead code"; §10 "delete rather than deprecate"; §B0 pristine foundation. The dead palettes also cost lazy-static allocations at first touch and rustdoc surface.
Fix: Delete util/palettes.rs (and its `pub mod palettes;` line + doc entry in util/mod.rs), `hex!`, `rgb!` (see finding 5), `get_font_source`, `get_some_font`, `_template.rs`, and the empty `mod test {}`. `FONT_SOURCES` + `do_for_all_sources` can then drop to `pub(crate)` (only `load_fonts` uses them) — if the enumerate-sources seam is wanted for plugins later (§7), reintroduce it deliberately with a shaped API, not via leftovers.
Effort: S

### 10. Unused dependencies: `syn`, `smol_str`, `serde-lexpr`, `enumset` in baumhard; unused direct `cosmic-text` in mandala
Severity: P2 | Category: dead-code | Confidence: high (baumhard four) / med (mandala cosmic-text)
Files: /home/user/mandala/lib/baumhard/Cargo.toml:12,15,18,27; /home/user/mandala/Cargo.toml:21
Evidence: grep across lib/baumhard/src + benches finds no `syn::`/`use syn`, no `SmolStr`, no `serde_lexpr`/`lexpr`, no `EnumSet`/`use enumset`. `syn 2.x` in particular is one of the heaviest compile-time dependencies in the Rust ecosystem and it sits in `[dependencies]` of a crate that also targets wasm32. In the app crate, `cosmic-text = "0.18.2"` is a direct dependency but no file in src/ contains a code-level `use cosmic_text`/`cosmic_text::` (only doc comments); the renderer consumes cosmic types exclusively through the `baumhard::font` re-export seam (font/mod.rs:44-89), which is exactly what that seam was built for.
Why it matters: §B1/§4 — compile time and WASM binary hygiene; a dependency list is also documentation, and this one lies about what the crate uses.
Fix: Remove the four from baumhard's Cargo.toml; remove mandala's direct cosmic-text dep (the lockfile already unifies the version through baumhard; if the intent was a version pin, a comment on baumhard's dep line is the honest spelling). Verify with `./build.sh` (native + wasm).
Effort: S

### 11. `replace_substring` copies the whole string twice and revalidates UTF-8 on the text-edit hot path
Severity: P2 | Category: performance | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:85-100; contrast :249-257 (`delete_grapheme_at`)
Evidence:
```rust
let mut bytes = s.as_bytes().to_vec();      // full copy #1
bytes.drain(i..n);
bytes.splice(i..i, source_bytes.iter().cloned());
if let Ok(modified_string) = String::from_utf8(bytes) {  // O(n) revalidation + move
    *s = modified_string;                    // old buffer dropped
}
```
`replace_graphemes_until_newline` (the per-keystroke write primitive of the glyph-matrix editor, per gfx_structs/model/matrix.rs:194) funnels every call through this. Both `i` and `n` come from `find_byte_index_of_grapheme`, which only ever returns char boundaries, so `s.replace_range(i..n, source)` is semantically identical, in-place, and skips the second allocation and the UTF-8 rescan. The sibling `delete_grapheme_at` already uses `replace_range` — same operation class, two different implementations and two different error postures (silent log-and-keep vs boundary panic). The function's own doc calls the allocation "a known hot-path allocation tracked alongside the rest of the 'no-alloc text edit' work" — §5 says do it, not track it.
Why it matters: §B7 no allocations in hot loops (`replace_graphemes_until_newline` is benched, making it hot-path by definition); §2 idiom repetition — one idiom per operation class.
Fix: Replace the body with `s.replace_range(i..n, source);` (boundaries are guaranteed by construction; if the defensive posture must survive, gate with `debug_assert!(s.is_char_boundary(i) && s.is_char_boundary(n))`). Run `./test.sh --bench` and quote the delta.
Effort: S

### 12. `split_off_graphemes` collects every grapheme into a `Vec<&str>` and rebuilds both halves
Severity: P2 | Category: performance | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:112-124
Evidence: `original.graphemes(true).collect::<Vec<&str>>()` + `left.concat()` + `right.concat()` — three allocations and a full re-copy of both halves, to do what `find_byte_index_of_grapheme(original, at)` + `original.split_off(byte)` does with a single allocation (the returned suffix) and no prefix rebuild. Doc admits the cost. Callers include `GlyphComponent::split` (component.rs:116) and line splitting (line.rs:318) — interactive editing paths.
Why it matters: §B7; the primitive is benched (`split_graphemes`), so the win is measurable.
Fix:
```rust
match find_byte_index_of_grapheme(original, at) {
    Some(byte) => original.split_off(byte),
    None => String::new(),
}
```
(Also fixes the current quirk where the `at >= len` path calls `original.split_off(original.len())` just to make an empty String.)
Effort: S

### 13. metric_cache allocates a `String` key per lookup — including hits — and its cost/size doc claims are wrong
Severity: P2 | Category: performance | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/font/metric_cache.rs:69 (CacheKey), :130,:156,:200 (`grapheme.to_string()` before lookup), :28 (":Hit ... O(1)"), :35 ("every entry is ~12 bytes"), :45-46 ("~100 ns each")
Evidence: `let key = (face, OrderedFloat(size_pt), grapheme.to_string());` runs before the read-lock probe, so every hit allocates and frees a heap String. `cluster_width` does it once per grapheme per rail per node per scene rebuild (border.rs:751-791). The module doc's "~12 bytes" per entry ignores the heap String + key tuple + map overhead (realistically 60-100+ bytes), and "~100 ns" ignores the allocation. Growth is unbounded in principle (process-lifetime, admitted "dead memory until process exit"); in practice keys are bounded by authored `font_size_pt` values since border sizes are not zoom-scaled at this layer — worth one honest sentence instead of a wrong number.
Why it matters: §B7 (no allocations in hot loops — this is the border hot path the module exists to accelerate); §B9 (cost claims a consumer would rely on are false).
Fix: Key on the borrowed form: use a two-level map or `hashbrown::HashMap` + `raw_entry` (or an `(Option<AppFont>, OrderedFloat<f32>)` outer key with an inner `FxHashMap<Box<str>, f32>` probed via `&str`), allocating only on miss. Correct the module doc's numbers while there.
Effort: M

### 14. `replace_graphemes_until_newline`: doc contract ("stops at the first \n in either string") and growth-return are wrong for multi-line sources
Severity: P2 | Category: docs | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:24-45 (doc), :46-68 (body); fixture rows exercising multi-line sources: /home/user/mandala/lib/baumhard/src/util/tests/grapheme_chad_tests.rs:123-136
Evidence: Empirically confirmed: `replace_graphemes_until_newline(&mut "xxxx", 0, "AB\nCD")` produces `"AB\nCD"` and returns `Some((0, 1))`. The source is inserted wholesale (newline and all) — the function does *not* stop at the source's `\n` — and the returned "extra = 1" (5 source graphemes − 4 line graphemes) is meaningless as a same-line region-shift: the first line actually *shrank* from 4 to 2 graphemes and a new line appeared. The test table (rows at :123-136) locks in the insert-everything behavior, so the code is the contract and the doc lies. Downstream, matrix.rs:194 feeds the return into `ColorFontRegions` range shifting — safe only while sources are single-line.
Why it matters: §B9 "a doc comment that lies about its function is worse than no doc comment at all"; the return value is load-bearing for region-index integrity (§B6).
Fix: Rewrite the doc to state the real contract ("`source` is inserted in full; only the *target* replacement window stops at the target's next newline; the returned growth is in graphemes and is only meaningful for single-line sources"), or — better per §B0 — make the contract real: debug_assert single-line source, or return a richer delta that region-shift callers can trust for multi-line input.
Effort: S

### 15. Raw `FONT_SYSTEM.write()` in `load_fonts` and an overclaiming helper doc; renderer `try_write` sites contradict "every call site" rule as written
Severity: P2 | Category: convention | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/font/fonts.rs:46, :309-311; /home/user/mandala/src/application/renderer/mod.rs:901,955; /home/user/mandala/src/application/renderer/render.rs:387 (plus metric_cache — finding 2)
Evidence: `acquire_font_system_write`'s doc: "Every `FONT_SYSTEM.write()` call site in the codebase should go through this helper instead of calling `RwLock::write` directly." Reality: `load_fonts` (fonts.rs:46) blocks on raw `.write()` (a re-entrant lazy-static init would hang without the timeout diagnostic — precisely the bug class `init()`'s own comment worries about); metric_cache has three more (finding 2); and three renderer sites use `try_write` + skip-frame — a *good* §9 degrade pattern, but one the doc's absolute claim doesn't acknowledge.
Why it matters: §B5; a discipline doc that the same file violates one page earlier trains readers to ignore it.
Fix: Route `load_fonts` through `acquire_font_system_write("load_fonts")`; amend the helper doc to name the two sanctioned shapes (blocking-with-timeout via this helper; non-blocking `try_write` + frame degrade in the renderer) so the rule matches the code.
Effort: S

### 16. Region→byte conversion re-walks the text from index 0 twice per region, once per shape pass
Severity: P2 | Category: performance | Confidence: med
Files: /home/user/mandala/lib/baumhard/src/font/attrs.rs:98-100 (attrs_list), :253-256 (rich_text_spans); /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:74-83
Evidence: Both bridges call `find_byte_index_of_grapheme(text, start)` and `...(text, end)` per region — each an O(n)-from-zero grapheme walk. `RegionFamilies` was introduced precisely to hoist per-pass work out of the 9-pass loop (main + 8 halo stamps, per its doc), but byte ranges are still recomputed inside `rich_text_spans_from_regions` on every pass: cost O(passes × regions × 2 × n) per text area per rebuild. Regions are submitted in ascending, non-overlapping order (`ColorFontRegions` contract), so a single forward walk can resolve all boundaries.
Why it matters: §B1/§B7 — this sits inside the renderer's per-area rebuild loop with the write guard held (scene_buffers.rs:64-74), so wasted walks extend lock scope too (§B5).
Fix: Resolve byte ranges once in `RegionFamilies::resolve` (store `Vec<(usize, usize)>` beside `names`, computed in one pass over `text` — resolve() already takes the text's regions; add the `text: &str` parameter) and have `rich_text_spans_from_regions` slice from the cache. Bench with `rich_text_spans_two_regions_slice_text_per_range` before/after.
Effort: M
### 17. `release_max_level_off` silently deletes the §9 "log and degrade" posture from every build users run
Severity: P2 | Category: error-handling | Confidence: med (may be a deliberate perf choice, but it is undocumented and contradicts §9's observable intent)
Files: /home/user/mandala/lib/baumhard/Cargo.toml:16; /home/user/mandala/Cargo.toml:32
Evidence: Both crates set `log = { ..., features = ["release_max_level_off"] }`, which compiles out `error!`/`warn!`/all levels in release. `./build.sh` and `./run.sh` ship release binaries; the WASM bundle is `trunk build --release`. Every degrade path in scope — `resolve_font_family`'s "unknown font id, dropping family pin" warns (attrs.rs:124,131,143), `replace_substring`'s "Failed to convert bytes to UTF-8 String" (grapheme_chad.rs:98), `build_family_index`'s empty-family warn (fonts.rs:149) — becomes a *silent* degrade in production. §9's contract is "Degrade the frame, log via log::warn!/log::error!, keep running"; the second half is compiled out exactly where the first half matters.
Why it matters: corrupt-save and font-drift conditions become undiagnosable from user reports; the codebase's own comments treat these warns as the observability story.
Fix: Decide and document. If the max-level-off is a deliberate mobile-perf choice, say so in CODE_CONVENTIONS §9 and treat warns as debug-build-only tooling. Otherwise switch to `release_max_level_warn` so error/warn survive release while the chatty debug!/trace! paths (e.g. fonts.rs:45,50) still vanish.
Effort: S

### 18. Trailing-line disagreement: `count_number_lines("abc\n") == 2` but line 1 is unaddressable by both `find_nth_line_*`
Severity: P2 | Category: correctness | Confidence: med
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:129-131, :165-168, :198-201; fixtures: /home/user/mandala/lib/baumhard/src/util/tests/grapheme_chad_tests.rs:65 ("abcd\n" → 2), :80 (("a\n", 1) → None)
Evidence: `count_number_lines` counts the empty trailing line ("the trailing line counts even when `s` does not end in `\n`", so `"abc\n"` = 2), but `find_nth_line_grapheme_range("abc\n", 1)` and the byte variant both return `None` — the final `(line_head == n && new_line)` arm rejects the empty last line. The fixtures pin both behaviors separately without confronting the contradiction. Any caller iterating `0..count_number_lines(s)` and calling `find_nth_line_*` per index gets a guaranteed `None` on the last iteration of every newline-terminated string — a cursor on the empty final line of a text area is exactly this shape.
Why it matters: §T1 fundamentals must compose; two primitives in the same file disagree about what a "line" is.
Fix: Make the finders return `Some((len, len))`-style empty ranges for the trailing empty line (matching the `("\n", 0) → Some((0,0))` precedent already in the fixtures), or change `count_number_lines`'s doc + behavior — one line model, both functions, fixtures updated together.
Effort: S

### 19. §B9 audit: high compliance overall, but the generated `AppFont` enum is undocumented (and its one doc line is profanity), struct fields lack docs, and there are three stale/contradictory doc fragments
Severity: P2 | Category: docs | Confidence: high
Files: /home/user/mandala/lib/baumhard/build.rs:13-14 (DOC_ANY_STR), :43-56 (no enum/variant docs emitted); /home/user/mandala/lib/baumhard/src/font/fonts.rs:394-395 (stale fragment), :425-432 (InkBounds fields); /home/user/mandala/lib/baumhard/src/font/metric_cache.rs:94-98 (InkExtent fields); /home/user/mandala/lib/baumhard/src/util/ordered_vec2.rs:23-24; /home/user/mandala/lib/baumhard/src/font/mod.rs:29-35 vs live callers; /home/user/mandala/lib/baumhard/src/util/tests/mod.rs, /home/user/mandala/lib/baumhard/src/font/tests/mod.rs
Evidence & ratio: Hand-written pub items in scope are ≈95% documented, with cost notes routinely present — exemplary against §B9. The gaps:
- Generated code: `pub enum AppFont` carries no `///` at all and no variant docs except `Any`, whose emitted doc is "Indicates that the defining party does not give two fucks about the font used" — public rustdoc, in a crate the project hopes to extract as a standalone library (§0's own bar).
- `InkBounds` (6 pub fields), `InkExtent` (3), `OrderedVec2` (`x`, `y`) — fields undocumented; sibling `TextBlockSize` documents all fields, so the crate's own precedent is inconsistent.
- fonts.rs:394-395: a stale doc fragment "Opaque black. The default foreground colour for newly-built `AttrsList`s." is fused onto the top of `InkBounds`'s doc — leftover from a const that moved to font/mod.rs (`COLOR_BLACK`).
- font/mod.rs:31-34 declares `metrics` "Deprecated in favour of `metric_cache::glyph_advance`", but metrics.rs itself carries no deprecation, has no `#[deprecated]`, and has five live app call sites (color_picker/compute_sizing.rs:91,96; renderer/borders.rs:43; console_geometry.rs:225; scene_buffers.rs:174). §10: this repo deletes rather than deprecates — either finish the migration or drop the claim.
- `pub mod` declarations in the two tests/mod.rs files have no docs (low value, but §B9 says "no exceptions").
Fix: Emit a real doc comment for the enum and per-variant docs (family name + source file) from build.rs, and replace DOC_ANY_STR with professional wording; delete the stale fragment; doc the struct fields; reconcile the metrics deprecation claim (either migrate the five callers to a face-aware path or reword mod.rs to describe the actual division of labor).
Effort: M

### 20. build.rs robustness: UTF-16BE name records can panic the build; sanitized-name collisions generate a duplicate-variant enum
Severity: P2 | Category: correctness | Confidence: med
Files: /home/user/mandala/lib/baumhard/build.rs:193-215 (`get_font_name`), :110-116 + 217-254 (camel_case + no variant-level dedup), :139-189 (`fallback_sanitize`)
Evidence: `std::str::from_utf8(name.name).expect("Not UTF-8")` reads the first FULL_NAME record raw. Windows-platform name records are UTF-16BE; pure-ASCII names survive by accident (interleaved NULs are valid UTF-8 and the later `is_ascii_alphanumeric` filter strips them), but any non-ASCII character (e.g. `é` = 0x00 0xE9) yields invalid UTF-8 and panics the whole build — the graceful `fallback_sanitize` path is unreachable for exactly the fonts that need it (use ttf-parser's UTF-16-aware `name.to_string()` instead). Separately, dedup is keyed on filename prefix only; two files with different prefixes but identical camel-cased *internal* names would emit `enum AppFont { X, X }` and fail the build with a confusing generated-code error. `fallback_sanitize(&filename)` also operates on the extension-bearing filename, so a numbers-only-named font would produce the invalid variant `.ttf`.
Why it matters: the quest question "what happens on weird dir names" — dir names are fine; weird *name tables* break the build with a panic instead of the designed fallback. §9's startup-`expect` rule doesn't cover making the fallback path unreachable.
Fix: Use `name.to_string()` (handles UTF-16BE), fall back to `fallback_sanitize` on `None`; dedup on the final variant name (append a numeric suffix or skip-with-warning on collision); strip the extension before fallback sanitizing.
Effort: S

### 21. Epsilon discipline: prod deviation in custom/sync.rs; missing f64 `almost_equal` pushes app tests to hand-roll tolerances
Severity: P2 | Category: ssot | Confidence: high (sites) / med (impact)
Files: /home/user/mandala/src/application/document/custom/sync.rs:260-261; /home/user/mandala/lib/baumhard/src/util/geometry.rs:22-28; test-side hand-rolls: src/application/document/mutations/tree_cascade.rs:167,248,256, flower_layout.rs:130,237,246 (all inside #[cfg(test)]), lib/baumhard/src/mindmap/model/tests.rs:121
Evidence: Both crates' production code is otherwise clean — the only epsilon constant in baumhard prod is the canonical `ERROR_TOLERANCE_ALMOST_EQUAL: f32 = 1e-5`, and the app's `f32::EPSILON` uses at renderer/hit.rs:214 and edges/structural.rs:266 are zero-guards, not equality tolerances (fine). The deviation: sync.rs:260-261 compares node sizes with `> f32::EPSILON` — for values in the hundreds this is effectively `!=` (EPSILON ≈ 1.2e-7 is below one ULP at 256.0), which is either an accidental exact-compare or a misspelled `pretty_inequal`. Test modules hand-roll `1e-6` on **f64** positions because `almost_equal` is f32-only; geometry.rs offers `is_non_negative_finite_f64` but no f64 equality helper — a missing primitive per §1 ("missing primitives are added to Baumhard").
Why it matters: THE stated point of `almost_equal` is single-sourcing "close enough" (CONCEPTS.md §2); sync.rs is one drift away from a heisenbug when sizes pass through f32↔f64 conversions.
Fix: sync.rs → `pretty_inequal` (or document why exact inequality is intended); add `almost_equal_f64` (+ `do_*` test + bench) to geometry.rs and migrate the test hand-rolls on the way past (§5 drive-by rule).
Effort: S

### 22. Test/bench gaps on in-scope primitives: `insert_spaces` untested; geometry helpers without `do_*`; `is_prime` ceiling can defeat the RegionParams prime guard
Severity: P2 | Category: testing | Confidence: high (gaps) / med (is_prime impact)
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:217-226 (`insert_spaces`); /home/user/mandala/lib/baumhard/src/util/geometry.rs:61-63 (`pretty_inequal`), :103-105 (`vec2_area`), :112-115 (`aabb_center`), :120-122 (`pretty_inequal_vec2`); /home/user/mandala/lib/baumhard/src/util/primes.rs:47-52; /home/user/mandala/lib/baumhard/src/gfx_structs/util/regions.rs:118-119,289-290
Evidence: `insert_spaces` is a pub grapheme primitive with **no test and no bench** — grapheme_chad_tests.rs doesn't even import it (§B3 requires do_* + bench in the same commit; §T1 makes grapheme handling a fundamental). Geometry: `pretty_inequal`, `pretty_inequal_vec2`, `vec2_area`, `aabb_center` have no dedicated `do_*` bodies (they're exercised only implicitly). `is_prime(n)` returns `false` for every `n > PRIME_CEILING = 10_000` (documented), and `RegionParams` guards with `assert!(!is_prime(resolution.0))` — a prime dimension above 10,000 (e.g. 10007 px) sails through the guard and lands in exactly the degenerate-grid case the prime table exists to prevent.
Why it matters: §T12 "when in doubt, write the test"; the prime guard silently weakening above an arbitrary ceiling is the kind of cliff §7's "never dismiss a use case as niche" warns about (large canvases are real).
Fix: Add `do_insert_spaces` (+ n=0, past-end, mid-emoji cases) with bench; add `do_*` for the four geometry helpers; make `is_prime` fall back to trial division above the ceiling (√n is ~100 iterations at 10^4-10^8 — trivially cheap at RegionParams-construction frequency), or assert `n <= PRIME_CEILING` so the limit is loud.
Effort: M

### 23. American English: 50 British spellings across 14 in-scope files (project mandates American)
Severity: P3 | Category: convention | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/font/mod.rs (colour, quantisation ×4), font/color.rs (quantise/colour), font/fonts.rs (Materialise :202, initialiser/initialisations :120-129, recognise :160, localised :231, rasterisation :606, behaviour :599), font/metrics.rs (sanitiser :44, honours :29), font/metric_cache.rs (rasterise ×3, behavioural :237), font/tests/attrs_tests.rs (honour, behaviour), font/tests/hex_tests.rs (colour ×2), util/color.rs (colour ×6), util/color_conversion.rs (Colour :3, colour ×3, neighbourhood :9), util/mod.rs (colour ×2, initialisation), util/log.rs (Initialise ×2), util/palettes.rs (colour ×3), bin/generate_stress_map.rs ("serialise mindmap" — a user-facing error string, :542; behaviour :9), benches/test_bench.rs ("behaviour" :77, bench name "region_indexer_initialise" :264 vs the American fn `do_region_indexer_initialize` it calls)
Evidence: 50 grep hits (list above is representative; the bench *name* / function-name split at test_bench.rs:264-266 means the recorded criterion baseline is spelled differently from the code symbol).
Why it matters: CLAUDE.md §6 "Use American English for consistency"; §2 "NEVER skip changes because they are merely cosmetic". The stress-tool error string and the criterion bench name are user-visible.
Fix: Mechanical sweep of the listed files (colour→color, -ise→-ize family, behaviour→behavior, honours→honors, neighbourhood→neighborhood); rename the criterion entry to `region_indexer_initialize` (accepting one baseline reset).
Effort: S

### 24. Naming and small-shape polish across scope
Severity: P3 | Category: api-design | Confidence: high
Files: /home/user/mandala/lib/baumhard/src/util/grapheme_chad.rs:376,399 (`delete_back_unicode`/`delete_front_unicode` vs the module's `*_grapheme(s)` vocabulary; `char_count` variables actually count bytes), :129 (`count_number_lines` grammar); /home/user/mandala/lib/baumhard/benches/test_bench.rs:300,304 (bench names `remove_prefix_unicode`/`truncate_unicode` match neither the primitives nor the test names); /home/user/mandala/lib/baumhard/src/font/fonts.rs:372 (`do_for_all_sources` — a production API wearing the `do_*` benchmark-body naming convention; it appears in the unbenched-`do_*` diff as noise), :30-36,44-56,383-391 (`return x;` trailing-return style in an otherwise expression-style file), :496 vs :617-619 (one measurement primitive sets `buffer.set_size(.., None, None)` explicitly, the sibling relies on the default); /home/user/mandala/lib/baumhard/src/util/ordered_vec2.rs:41-46 (`new` wraps already-wrapped `OrderedFloat` in `OrderedFloat::from` again); /home/user/mandala/lib/baumhard/src/bin/generate_stress_map.rs:14-15 (module doc says skewed spine has "depth `--nodes`"; code builds `nodes/2` — fn doc :344 is correct) and `gen_skewed` off-by-one: odd `--nodes` yields `nodes − 1` nodes (spine `n/2` + at most `n/2` leaves; `gen_skewed(9)` → 8 nodes; test only checks the even case :597-600); /home/user/mandala/lib/baumhard/build.rs:71,191 (mid-file `use` statements), :111-116 (`let x; if cond { x = a } else { x = b }` instead of `match`/`unwrap_or_else`), :18 (`VALID_EXTENSIONS` const exists but :84 hardcodes the strings and :119 indexes it as `[0]`/`[1]` cryptically)
Evidence: as cited per site.
Why it matters: §2 "repetition of idiom is the point"; §B10 pub names are plugin-facing surface; the `do_*` collision actively pollutes the §B8 bench-audit tooling surface.
Fix: Rename `delete_back_unicode`/`delete_front_unicode` → `delete_back_graphemes`/`delete_front_graphemes` (repo-wide, no deprecation per §10) and align the two bench names; rename `do_for_all_sources` → `for_each_font_source` (or drop it to pub(crate) per finding 9); fix the stress-tool module doc + odd-count off-by-one (+ an odd-`nodes` test row); tidy the build.rs shapes while touching it for finding 8/20.
Effort: M

---

## Checked and found CLEAN

- **cosmic-text import boundary (§1/§B5)**: zero code-level `use cosmic_text`/`cosmic_text::` outside `lib/baumhard/src/font/` and `src/application/renderer/` across the whole workspace — every other grep hit is a doc comment. The renderer itself consumes cosmic types via the `baumhard::font` re-export seam.
- **App-crate color re-implementation**: none. No hex parsing, no HSV math, no byte↔float conversion outside baumhard (only `ColorValue::parse`'s shape check, noted in finding 5d).
- **Epsilon single-sourcing in baumhard prod code**: exactly one constant (`1e-5` in geometry.rs); no stray tolerances outside test modules, and test-side named epsilons are §T5-sanctioned scale choices.
- **attrs bridges Unicode correctness**: region→byte slicing verified grapheme-correct against ZWJ family emoji and regional-indicator flags, with clamping for out-of-range/corrupt regions (attrs_tests.rs covers all of it, including the zero-width-drop × color-override interaction).
- **`acquire_font_system_write` design**: timeout + poison messages carry the call-site tag; the timeout path has a real regression test that pins "panics, not hangs" with a should_panic(expected) message; the busy-wait-vs-try_write tradeoff is documented and sound for parallel `cargo test`.
- **`build_family_index` re-entrancy ordering**: the forced `COMPILED_FONT_ID_MAP` init before taking the read lock (fonts.rs:117-130) correctly closes the read-then-write self-deadlock, and `init()` documents the eager-init rationale.
- **test_bench.rs compiles**: `cargo bench -p baumhard --no-run` exits 0 with zero warnings (2m44s); every import path and every referenced `do_*` resolves.
- **generate_stress_map**: deterministic per seed (tested), serialization round-trip tested, clean arg parsing and error paths, O(N²) long-edge scan is documented and appropriate for a tool.
- **format/json.rs**: coherent single-purpose facade; doc names its consumers and the maptool carve-out; no drift.
- **util/{arena_utils, log, ordered_vec2, primes}** logic: `clone_subtree` recursion is correct and benched; logger init is properly cfg-split; `OrderedVec2` trait impls are consistent; the sieve is correct within its ceiling.
- **No `unsafe`, no TODO/FIXME/HACK** anywhere in scope.
- **Doc cost-notes (§B9)**: hand-written pub items in scope are ≈95% documented with genuine cost sections — several (fonts.rs measurement primitives, attrs.rs bridges, geometry.rs) are exemplary.
- **Font-file dedup collisions today**: all four (`treeroot`, `pictopeople`, `appletea`, `arigatou`) are otf+ttf pairs, which the ttf-preference rule resolves deterministically (the residual same-extension risk is in finding 8).
