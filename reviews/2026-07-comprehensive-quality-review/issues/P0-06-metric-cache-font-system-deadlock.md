# P0-06: `metric_cache` blocking-acquires FONT_SYSTEM internally — latent same-thread self-deadlock under the renderer's guard

**Severity:** P0-adjacent P1 (permanent hang when triggered; trigger is ordering-dependent today) · **Area:** baumhard/font · **Verified:** structural violation confirmed

## Context

CONVENTIONS §B5: `FONT_SYSTEM` is a global `RwLock`; every write acquire must go through `acquire_font_system_write(site)` — a timeout-guarded helper whose stated purpose (`lib/baumhard/src/font/fonts.rs:306-332`) is turning same-thread re-entrant acquires (guaranteed deadlock on a single-threaded app) into a diagnostic panic: "Every `FONT_SYSTEM.write()` call site in the codebase should go through this helper."

## Problem

`lib/baumhard/src/font/metric_cache.rs:214-216, 252-254, 282-284` — `shape_advance` / `shape_ink_height` / `shape_ink_extent` all do a **raw** `FONT_SYSTEM.write().expect(...)` with no timeout.

Meanwhile the renderer holds the write guard across whole rebuild loops and calls into the cache from inside them:

```rust
// src/application/renderer/scene_buffers.rs:34-38
let mut font_system = fonts::acquire_font_system_write("rebuild_border_buffers");
for elem in border_elements {
    let specs = baumhard::mindmap::border::border_run_specs(...)  // -> glyph_ink()/glyph_advance()
```

On a cache **miss** inside such a loop, `glyph_advance`/`glyph_ink` write-acquire the lock the same thread already holds: on Linux's futex RwLock that is a **permanent hang** (FreezeWatchdog abort after 10s on native; frozen tab on WASM).

It doesn't fire today only by ordering accident: the tree-builder border path usually warms the same cache keys first, outside any guard. Nothing guarantees that — the flat and tree border paths are independent, and a style/size/grapheme first seen under the guard measures cold.

Also in the same family: `load_fonts` (`fonts.rs:46`) blocks on raw `.write()`, and the helper's "every call site" doc doesn't acknowledge the renderer's three legitimate `try_write`+degrade sites (`renderer/mod.rs:901,955`, `render.rs:387`).

## Fix plan

1. **Preferred (composable, §B5's own precedent):** add `_with(font_system: &mut FontSystem, ...)` variants to the metric-cache API and thread `&mut FontSystem` through `border_run_specs` and other guard-holding callers — exactly the design `measure_glyph_ink_bounds`/`measure_text_block_unbounded` already use ("so the primitive composes with existing call sites that already hold the write guard"). Keep thin lock-acquiring wrappers for unlocked callers.
2. **Minimum:** replace the three raw `.write()` calls with `acquire_font_system_write("metric_cache::shape_advance")` etc., so a future re-entrant miss dies loudly with a site name instead of freezing. Do the same for `load_fonts`.
3. Amend `acquire_font_system_write`'s doc to name the two sanctioned shapes (blocking-with-timeout via helper; non-blocking `try_write`+frame-degrade in the renderer).
4. Test: a regression test that calls `glyph_advance` for an uncached key while holding the write guard must panic with the site-tagged timeout message (mirror the existing `should_panic(expected=...)` re-entrancy test), not hang.

## Acceptance criteria

- No raw `FONT_SYSTEM.write()` outside `acquire_font_system_write` (grep-clean).
- Guard-holding renderer loops can measure cold keys without deadlock (variant 1) or die diagnostically (variant 2).
- `./test.sh` green.

## Pointers

`lib/baumhard/src/font/metric_cache.rs`; `lib/baumhard/src/font/fonts.rs:46,306-332`; `src/application/renderer/scene_buffers.rs:34-42`; `lib/baumhard/src/mindmap/border.rs:501-538,751-791`; CONVENTIONS §B5, CODE_CONVENTIONS §9.
