# P1-33: Renderer re-shape granularity — picker hover re-shapes ~520 buffers, console keystroke re-shapes the whole console, resize drags re-shape the entire arena, halo spans recomputed 8×

**Severity:** P1 (the expensive half of "mutation, not rebuild" is not realized; steady-state mobile cost) · **Area:** mandala/renderer

## Problem

The §B2 dispatch (signature → InPlaceMutator vs FullRebuild) is correctly single-sourced in `AppScene`, and mutators mutate arenas in place — but **every** overlay mutator apply is followed by a full re-shape of every overlay glyph:

1. **Picker hover** (`overlay_dispatch.rs:64-85,135-164`): `apply_dynamic_mutator` then `rebuild_overlay_scene_buffers(app_scene)` — which shapes a fresh `cosmic_text::Buffer` per non-empty area (`tree_buffers.rs:187-221`). The picker has 58 live areas with `outline_px > 0`, so `shape_one_element_into_buffers` emits 1 main + 8 halo buffers each ≈ **520 buffer creations + set_rich_text + shape_until_scroll per hover change**, under one FONT_SYSTEM write guard, at mouse-move cadence. The gap is acknowledged in-code ("the cosmic-text shape pass is still per-element, which is the §B1 perf gap") — per CLAUDE.md §5 an acknowledged deferral is still the finding.
2. **Console keystroke**: each InPlaceMutator arm re-shapes all border/scrollback/completion/prompt areas; `console_overlay_areas` re-measures `measure_max_glyph_advance` (2 cosmic-text shapings + scratch buffer) on **every call** (`console_pass.rs:127`).
3. **Halo spans**: `rich_text_spans_from_regions` recomputed 8× with identical inputs per element per shape pass (`renderer/tree_walker.rs:185-204`); each call re-scans grapheme boundaries (`font/attrs.rs:240-255`). Related baumhard-side: the bridges re-run `find_byte_index_of_grapheme` from index 0 twice per region per pass — `RegionFamilies::resolve` should precompute byte ranges once (add the `text` param, store `Vec<(usize,usize)>`).
4. **Node/section resize drags** call `renderer.rebuild_buffers_from_tree(&tree.tree)` — full-arena re-shape of every text buffer once per drained frame (`throttled_interaction/section_resize.rs:133`, `node_resize.rs:108`) — while the move-drag path shows the intended shape (`patch_drag_positions` + background rebuild, no shaping) and the keyed `reshape_buffer_for(arena_id, tree)` already exists for targeted re-shaping.
5. **`mindmap_buffers` keyed by stringified usize** (`renderer/mod.rs:344-355`, `tree_buffers.rs:43,139`): every key is `usize::to_string()` — one String alloc per element per rebuild AND per moved node per drained drag frame; the doc justification ("Dewey-decimal addressing") is false — no Dewey string ever reaches this map.

## Fix plan

1. Make the buffer pass mutation-granular: report changed channels/ids from `apply_overlay_mutator` (or via the GlyphTreeEvent side channel) and re-shape only those elements into a keyed store — the `reshape_buffer_for` pattern generalized. Apply to picker + console.
2. Cache `measure_max_glyph_advance` per (glyph-set, font-size).
3. Hoist halo span construction out of the 9-stamp loop; precompute region byte-ranges in `RegionFamilies::resolve`.
4. Resize drains: re-shape only the affected container + its sections (loop `reshape_buffer_for`, or add `reshape_subtree_for`).
5. `mindmap_buffers: FxHashMap<usize, Vec<MindMapTextBuffer>>`; delete the stale justification.
6. Fold in the drag-patch halo landmine while touching `patch_drag_positions` (`tree_buffers.rs:118-146`): store per-buffer emission offsets so future outlined mindmap elements don't collapse their halos on drag (currently documented-latent).

## Acceptance criteria

- Picker hover and console keystroke re-shape only changed elements (add counters in a test or measure via bench; quote before/after).
- Resize drag cost scales with the resized node's section count, not the map.
- No String keys in the buffer map.
- `./test.sh` green; visual smoke on native + WASM.

## Pointers

`src/application/renderer/{overlay_dispatch.rs,tree_buffers.rs,console_pass.rs,tree_walker.rs,mod.rs}`; `lib/baumhard/src/font/attrs.rs` (RegionFamilies); CONVENTIONS §B1/§B2/§B5/§B7.
