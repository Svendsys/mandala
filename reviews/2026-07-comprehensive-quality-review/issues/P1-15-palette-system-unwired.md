# P1-15: The palette color system is documented as the render-time cascade but is unwired — `resolve_theme_colors` has no production caller

**Severity:** P1 (documented flagship format feature is inert) · **Area:** baumhard/mindmap model + scene/tree builders

## Problem

`format/palettes.md:52-64` documents the render-time cascade: "How a node resolves its colors: `resolve_theme_colors(node)` … the renderer falls back to the node's plain `style` colors", and promises "Editing a palette is a single-point change; every node using it updates on the next render." CONCEPTS §3 repeats it. The palette hoist is presented as the format's answer to the legacy per-node duplication (100× file-size reduction).

Reality: repo-wide grep for `resolve_theme_colors` / `color_schema` finds **no scene_builder / tree_builder / renderer caller** — node fill, frame, and text colors come exclusively from `node.style.*` + `resolve_var` (`scene_builder/node_pass.rs:152`; `tree_builder/node.rs:80-114,173-188`). `map.palettes` is consulted at render time only for border glyph-cycling (`resolve_palette_cycle`, `border.rs:1393`). `resolve_theme_colors` (`model/mod.rs:183-192`) is a pub API whose only caller is its own test.

Themed maps still look right only because the miMind converter baked resolved colors into `style.*` at migration time. Editing `map.palettes` today changes nothing on screen — the "single-point retheme" promise is false.

Two adjacent latent bugs in the resolver itself (fix while wiring):
- `level: i32` on `ColorSchema` (`node.rs:602`) but `schema.level as usize` (`model/mod.rs:186`) — negative levels wrap to huge values and clamp to the last group. CONCEPTS types it `usize`. Decide the type/semantics.
- The clamp branch (`groups.last()`) has zero test coverage.

## Fix plan (option a — wire it; the honest close per §5)

1. In `tree_builder/node.rs` and `scene_builder/node_pass.rs`, resolve node colors through `resolve_theme_colors(node)` when `node.color_schema` is `Some`, with `style.*` as the no-schema fallback — exactly as palettes.md describes. Thread `connections_colored` into the edge color cascade (see the edge-cascade helpers issue P1-26).
2. Respect `starts_at_root` and level clamping per format/palettes.md; add tests: schema'd node renders group colors; palette edit changes render output; out-of-range level clamps; `starts_at_root` both ways.
3. Fix the `level` type (u32/usize if never negative — update format docs + fixtures same commit per §10).
4. Add a `theme_demo`-style fixture exercising a live palette (the existing one relies on baked styles).

## Fix plan (option b — if wiring is deliberately deferred)

Rewrite palettes.md + CONCEPTS to state `color_schema` is carried-but-unconsulted today and mark `resolve_theme_colors` as the seam; add a `maptool verify` note. (Option a is strongly preferred: the data model, docs, and migration story all already assume it.)

## Acceptance criteria

- Editing `map.palettes` changes rendered node colors on next rebuild (option a), demonstrated by test.
- No doc claims a cascade the code doesn't perform.
- `./test.sh` green.

## Pointers

`lib/baumhard/src/mindmap/model/mod.rs:183-192`; `lib/baumhard/src/mindmap/model/palette.rs`; `lib/baumhard/src/mindmap/tree_builder/node.rs:80-114`; `lib/baumhard/src/mindmap/scene_builder/node_pass.rs:150-170`; `format/palettes.md`; CONCEPTS §3 (Palettes).
