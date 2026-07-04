# P1-34: Renderer's flat hitbox maps re-implement hit-testing the canvas trees already provide — and return hash-iteration-order winners on overlap

**Severity:** P1 (parallel spatial index, hand-synced; nondeterministic input routing) · **Area:** mandala/renderer + baumhard Scene

## Problem

Portal glyphs and connection labels already live in canvas trees (`CanvasRole::Portals`/`ConnectionLabels`; `renderer/mod.rs:377-380` confirms "Portal glyph buffers themselves flow through canvas_scene_buffers via the tree pipeline"), and baumhard provides memoized-AABB + BVH resolution over exactly those trees (`Scene::component_at`, `Tree::descendant_at/near` — smallest-area-wins, shape-aware, slack-capable).

The renderer nevertheless maintains three parallel flat indexes — `portal_icon_hitboxes`, `portal_text_hitboxes`, `connection_label_hitboxes` (`renderer/hit.rs:17-29,61-147`) — populated wholesale via setters at every portal/label rebuild (`scene_rebuild.rs:616,693-699`; :616 itself calls this "the legacy `Renderer::hit_test_portal`") and scanned linearly by `find_first_aabb_hit`.

Second defect: `find_first_aabb_hit` returns the **first hash-map key whose AABB contains the point** — over `FxHashMap`, "first" is arbitrary iteration order. Two different portals' icons, or labels on crossing edges, can overlap; click routing then depends on hash order — nondeterministic and untestable (icon-vs-text overlap is argued away in comments at hit.rs:140-144, but cross-element overlap is not).

`scene_host.rs:421-425` acknowledges the split as deferred work.

## Fix plan

1. Resolve clicks through `app_scene.canvas_scene()` + `component_at` with a role filter; the tree builder owns the channel→(EdgeKey, endpoint) identity mapping (it already assigns stable channels per portal pair/endpoint — see the channel-stability contract tests).
2. Delete the three maps + their setters + `find_first_aabb_hit`.
3. Until the migration lands (if staged), make the interim deterministic: smallest-area-wins or sorted iteration in `find_first_aabb_hit`.
4. Tests: overlapping-portal-icons fixture asserting deterministic, layer/area-principled selection; port existing portal/label hit tests to the tree path.

## Acceptance criteria

- One spatial-index implementation for canvas-space hit-testing (BVH); grep shows no flat hitbox maps.
- Overlap routing is deterministic and pinned by test.
- Portal/label click, drag-start, and double-click behaviors unchanged (existing tests green).

## Pointers

`src/application/renderer/hit.rs`; `src/application/app/scene_rebuild.rs:585-705`; `lib/baumhard/src/gfx_structs/scene.rs:257-286`; `lib/baumhard/src/gfx_structs/tree.rs:292-318`; CODE_CONVENTIONS §1 ("reach for the existing seam"), §2.
