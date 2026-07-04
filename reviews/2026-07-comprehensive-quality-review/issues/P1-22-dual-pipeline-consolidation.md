# P1-22: Dual-pipeline consolidation — the flat RenderScene consumers are dead code, yet every frame still pays for full dead element emission and double style resolution

**Severity:** P1 (architecture debt + the single largest per-frame waste) · **Area:** baumhard scene_builder/tree_builder + mandala renderer

## Context

CODE_CONVENTIONS §3 acknowledges the flat scene intermediary as "a consolidation seam, not a permanent second pipeline". The review found consolidation is **further along than the docs admit** — and the leftover half is now dead weight rather than a live second path.

## Current state (verified per-role matrix)

Every visual role reaches the GPU through `Tree<GfxElement, GfxMutator>`. The flat renderer consumers are dead: `rebuild_border_buffers`, `rebuild_edge_handle_buffers`, `rebuild_connection_label_buffers` (`src/application/renderer/scene_buffers.rs:26,91,126`) and `fit_camera_to_scene` (`renderer/hit.rs:33`) have **zero call sites**; their target buffer fields are initialized empty and never filled, yet still iterated in the draw chain (`render.rs:321-333`). Baumhard-side, `BorderStyle::top_text`/`bottom_text` + `build_horizontal_text` have no production callers either.

Yet `build_scene_with_cache` still manufactures, **per scene build** (every drag drain frame, picker-hover frame, zoom tick):

- One `TextElement` per non-empty section with `text: section.text.clone()` + full `Vec<TextRun>` clone + per-run color resolve (`scene_builder/node_pass.rs:229-280`) — dead output.
- Full `BorderStyle` resolution (~12 String allocs) + `resolve_palette_cycle` per framed node → `BorderElement` (node_pass.rs:286-312) — dead output, and then `border_node_data` **resolves the identical styles a second time** for the live border tree (`tree_builder/border.rs:100-131`).
- Per-endpoint portal style + text style + two layouts → `PortalElement` (portal.rs:387-478, which admits "exists for tests…not for the GPU") — dead, then `portal_pair_data` resolves it all **again** (`tree_builder/portal.rs:195-311`).
- `background_color` re-resolved to a fresh String (builder.rs:336) — renderer reads `Canvas.background_color` directly.

On a 250-node/300-edge map: ~250 full text clones + 250 run-vec clones + F×12 border strings + 2P portal styles of pure garbage per frame during interactions, plus the double resolution. (§B1/§B7 hot-path rules.)

## Fix plan — four independently-shippable steps

1. **Delete the dead flat consumers** (zero risk): the three renderer methods + empty buffer fields + draw-chain hooks; `fit_camera_to_scene`; baumhard's `top_text`/`bottom_text`/`build_horizontal_text`. ⚠ Port `line_height_pt` handling to the tree path FIRST (see P1-12 — the dead code is the only executable reference).
2. **Stop dead emission**: split `build_node_elements` into a clip-AABB pass (all the connection pass needs from the node walk — only resolved `font_size_pt` from the border cascade) and drop `TextElement`/`BorderElement`/`PortalElement`/`background_color` from `build_scene_with_cache`. Port affected scene tests to tree outputs (`portal_pair_data`, border tree). This deletes the double-resolve for free.
3. **Move the couriers home**: `SectionFrameElement`, `ConnectionLabelElement`, and handle emission move next to their tree builders (section_frame already demonstrates the single-resolution courier model). `RenderScene` shrinks to `connection_elements` + `edge_handles`.
4. **Final seam**: fold cache-aware connection sampling into `tree_builder::connection` (`SceneConnectionCache` survives unchanged; only the element struct moves); delete `RenderScene`; update CODE_CONVENTIONS §3 + CONCEPTS §3 to describe the converged pipeline.

Blockers per step are only tests + the node_pass AABB coupling; no interactive feature depends on flat outputs.

## Acceptance criteria

- Step 1: grep-clean for the dead methods/fields; `./test.sh` green; visual smoke via `./run.sh`.
- Step 2: per-frame allocations in `build_scene_with_cache` drop to connection-pass needs only (verify with a before/after `./test.sh --bench` on scene-build benches, quote numbers per §B6-style discipline).
- Steps 3-4: RenderScene gone; docs updated in the same commits.

## Pointers

Full matrix + evidence: `reviews/2026-07-comprehensive-quality-review/README.md` (Projection section) and the projection findings file. Key files: `lib/baumhard/src/mindmap/scene_builder/{builder.rs,node_pass.rs,portal.rs}`; `lib/baumhard/src/mindmap/tree_builder/{border.rs,portal.rs}`; `src/application/renderer/{scene_buffers.rs,hit.rs,render.rs,mod.rs}`; `src/application/app/scene_rebuild.rs`.
