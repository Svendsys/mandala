# P2-38: grapheme_chad API gaps — app crate hand-rolls word/line/clip primitives with direct unicode-segmentation; `replace_graphemes_until_newline`'s contract is wrong; `insert_spaces` untested

**Severity:** P2 (§1/§B3 discipline: missing primitives belong in Baumhard) · **Area:** baumhard/util + mandala call sites

## Problems

1. **App-crate direct `unicode_segmentation` uses that re-implement existing primitives**: `section_structure.rs:311,332` (`graphemes(true).count()` ≡ `count_grapheme_clusters`), :344-354 (`grapheme_indices(true).nth(g)` ≡ `find_byte_index_of_grapheme`).
2. **Missing primitives hand-rolled at 5+ sites**: backward word-boundary scan duplicated between `console_input/edit.rs:169-195` (`kill_word`) and `console_input/completion.rs:107-127` (near-identical loops; both allocate `Vec<&str>` of all clusters — the exact allocation `word_left`'s doc says it was created to eliminate; semantics differ: whitespace-delimited vs alphanumeric, so a `word_left_ws` sibling is needed, not a drop-in); line-boundary walks in `text_edit/mod.rs:166-197`; two different first-N-clusters truncations **in one file** (`console/commands/section/mod.rs:180-212` double-walks take(20)+count; :464-480 single-pass ≤40 — the second's comment condemns exactly what the first does); `accept_console_completion` collects a slice-Vec per Tab.
3. **Baumhard-internal duplicates next door to grapheme_chad**: `border.rs:1508-1510` `count_clusters` is byte-for-byte `count_grapheme_clusters`; `border_pattern.rs:315-321` has a third private splitter.
4. **`replace_graphemes_until_newline` doc/return contract wrong for multi-line sources** (empirically confirmed): doc says "stops at the first \n in either string" — the source is inserted **wholesale** (newline included), and the returned growth (source_clusters − line_clusters) is meaningless as a same-line region shift when the source is multi-line (first line actually shrank; a new line appeared). The return feeds ColorFontRegions range-shifting (`matrix.rs:194`) — safe only while sources are single-line, which nothing enforces.
5. **`insert_spaces` has no test and no bench** (`grapheme_chad.rs:217-226`) — a pub grapheme primitive, violating §B3's same-commit rule. Also an empty `#[cfg(test)] mod test {}` at the file tail.

## Fix plan

1. Add to grapheme_chad (each with `do_*` test + bench, §B3): `take_graphemes(&str, n) -> (&str, bool)`; `prev_word_boundary_ws` (whitespace-delimited) alongside the existing `word_left`; `line_bounds_at`; keep signatures allocation-free.
2. Migrate the listed app sites + baumhard-internal duplicates; then remove mandala's direct `unicode-segmentation` dependency (compiler-enforced containment, mirroring the cosmic-text plan).
3. Fix `replace_graphemes_until_newline`: either make the contract real (debug_assert single-line source; or return a richer delta callers can trust) or rewrite the doc to the actual behavior and audit the region-shift caller.
4. Tests for `insert_spaces` (n=0, past-end, mid-emoji); delete the empty test module.
5. Fold in the two perf rewrites from P2-36 item 8 if not already landed (same file, same benches).

## Acceptance criteria

- `grep -rn "unicode_segmentation" src/` returns nothing (dep removed from mandala's Cargo.toml).
- One truncation/word/line implementation each, in baumhard, benched.
- `./test.sh` green; `./test.sh --bench` green.

## Pointers

CONVENTIONS §B3 ("All text primitives live in grapheme_chad.rs"; "New text primitives go in grapheme_chad.rs" with same-commit test+bench); CODE_CONVENTIONS §1; files cited inline.
