# Mandala renderer-side review — findings

Scope: src/application/renderer/ (all), scene_host.rs, color_picker/ (all), color_picker_overlay/ (all), widgets/, frame_throttle.rs. Every file read end to end, including tests. Baumhard counterparts (gfx_structs/tree_walker.rs, scene.rs, shape.rs, font/fonts.rs, font/color.rs, util/color*) read for the comparison quests.

## Architecture assessment

The renderer is in a deliberate, well-documented mid-migration state and the migration discipline is mostly excellent: the §B2 dispatch (signature → InPlaceMutator vs FullRebuild) is single-sourced in `AppScene`, the picker's layout/dynamic two-phase mutator design with JSON-declared channel layout is genuinely sophisticated, hot-path caches (axis-split HSV tables, selection-rect shape cache, FPS/mode-status re-shape short-circuits) are thoughtful, FONT_SYSTEM lock discipline is consistent (frame path try-locks and degrades; rebuild paths block-with-timeout; the 9x halo stamping runs under one acquisition), and the pure-math test posture honors §T8 with strong round-trip pinning. The two structural weaknesses are: (1) the §B2 story stops at the arena — every overlay mutator apply is followed by a full re-shape of every overlay glyph (`rebuild_overlay_scene_buffers`), which for the halo'd picker means ~520 cosmic-text buffer creations per hover change, so the expensive half of "mutation, not rebuild" is not yet realized; and (2) a residue ring of half-retired surfaces — dead `RedrawMode` arms hiding a `Duration`-underflow panic, a never-written `console_overlay_buffers` field, dead decree variants, retired picker title/hint/chip plumbing still allocating per rebuild — plus a notable density of stale doc comments (including one that mis-describes the shader's `shape_id` encoding). Hardcoded `Bgra8UnormSrgb` for the atlas/rect pipeline while the surface config takes `capabilities.formats[0]` is the one latent correctness risk.

## Findings

### R1. Swapchain format has two sources of truth (hardcoded `Bgra8UnormSrgb` vs `capabilities.formats[0]`)
Severity: P1 | Category: correctness | Confidence: med
Files: src/application/renderer/mod.rs:600-613, mod.rs:677-681, src/application/renderer/pipeline.rs:11-19, Cargo.toml:18
Evidence:
```rust
let swapchain_format = TextureFormat::Bgra8UnormSrgb;          // mod.rs:600 (hardcoded)
let texture_format = surface_capabilities.formats[0];          // mod.rs:602
let config = Self::create_surface_config(texture_format...);   // surface uses formats[0]
let mut atlas = TextAtlas::new(&device, &queue, &glyphon_cache, swapchain_format); // atlas uses hardcoded
...
targets: &[Some(wgpu::ColorTargetState { format: swapchain_format, ... })]         // rect pipeline uses hardcoded
```
The render pass targets the surface texture view (format = `formats[0]`), while the glyphon `TextAtlas` and the rect pipeline are built for hardcoded `Bgra8UnormSrgb`. wgpu requires pipeline color-target format == pass attachment format; a mismatch is a fatal per-draw validation error. This works only because `formats[0]` happens to be `Bgra8UnormSrgb` on the desktop backends tested. Cargo.toml enables the `webgl` feature, and wgpu's GL/WebGL2 backend commonly reports `Rgba8UnormSrgb` first — the WASM/mobile-browser deployment (§4 first-class) is where the accidental agreement is most likely to break. Also `texture_format.clone()` (mod.rs:604) clones a `Copy` enum.
Why it matters: CODE_CONVENTIONS §4 (three first-class deployments). A format mismatch is not a degraded frame; it is a black screen / fatal validation error on the affected backend. Two sources of truth for one value is the SSOT smell even where it currently works.
Fix: Derive one format — `let swapchain_format = surface_capabilities.formats[0];` (or first-sRGB-with-fallback) — and feed that single value to surface config, `TextAtlas::new`, and the rect pipeline. Drop the `.clone()`.
Effort: S

### R2. §B2 overlay mutators still trigger a full overlay re-shape — picker hover re-shapes ~520 buffers, console keystroke re-shapes the whole console
Severity: P1 | Category: performance | Confidence: high
Files: src/application/renderer/tree_buffers.rs:187-221, src/application/renderer/overlay_dispatch.rs:64-85, 135-164, src/application/renderer/tree_walker.rs:185-204, src/application/renderer/console_pass.rs:127, src/application/widgets/color_picker.json:17 (outline_px 2.5)
Evidence: every mutator arm ends with a full re-walk:
```rust
// overlay_dispatch.rs:162-163 (per-frame hover path)
color_picker::apply_dynamic_mutator(app_scene, geometry, layout);
self.rebuild_overlay_scene_buffers(app_scene);   // re-shapes EVERY overlay glyph
```
`rebuild_overlay_scene_buffers` shapes a fresh `cosmic_text::Buffer` per non-empty area (tree_buffers.rs:204-219). The picker has 58 live areas and `outline_px > 0`, so `shape_one_element_into_buffers` emits 1 main + 8 halo buffers per area (tree_walker.rs:185-204) — roughly 520 `buffer::create` + `set_rich_text` + `shape_until_scroll` calls under one FONT_SYSTEM write guard per hover-state change (mouse-move cadence, throttled only adaptively by `color_picker_hover`). The console path has the same shape: each keystroke's InPlaceMutator arm re-shapes all border/scrollback/completion/prompt areas, and `console_overlay_areas` re-measures `measure_max_glyph_advance` (2 cosmic-text shapings + a scratch buffer) on every call. The gap is acknowledged in-code ("the cosmic-text shape pass is still per-element, which is the §B1 perf gap", overlay_dispatch.rs:131-134) — but per CLAUDE.md §5, an acknowledged deferral of the hard part is still the finding. Sub-point: for halos, `rich_text_spans_from_regions` is recomputed 8x with identical inputs per element (tree_walker.rs:193-197); each call re-scans grapheme boundaries over the text (font/attrs.rs:240-255).
Why it matters: §B1/§4 mobile budget — this is the steady-state cost of hovering the picker or typing in the console on a phone. The dynamic-mutator machinery (JSON dynamic spec, axis-split HSV caches, `PickerDynamicApplyKey` short-circuit) exists precisely to make this path cheap, and the shaping pass then discards the win.
Fix: Make the buffer pass mutation-granular: report changed channels from `apply_overlay_mutator` (or via the existing `GlyphTreeEvent` side channel) and re-shape only those elements into a keyed buffer store — the exact pattern `reshape_buffer_for` already implements for mindmap elements (tree_buffers.rs:76-107). Cache `measure_max_glyph_advance` per (glyph-set, font_size). Hoist halo span construction out of the stamp loop and clone per stamp.
Effort: L

### R3. Dead `RedrawMode` machinery in `process()` contains a `Duration`-underflow panic
Severity: P2 | Category: dead-code | Confidence: high
Files: src/application/renderer/mod.rs:841-873, mod.rs:847, mod.rs:711-713, 718, src/application/renderer/decree.rs:48-50, src/application/common.rs:22-26
Evidence: `redraw_mode` is set exactly once, to `NoLimit` (mod.rs:718); no setter exists anywhere (grep: construction + match sites only). The `OnRequest` and `FpsLimit(_)` arms of `process()` are unreachable, as is decree.rs:48's `if self.redraw_mode == RedrawMode::OnRequest { self.render(); }`. The dead `FpsLimit` arm holds a landmine:
```rust
let delta_duration = self.target_duration_between_renders - self.last_render_time; // mod.rs:847
```
`Duration::sub` panics on underflow; `target_duration_between_renders` is 10ms and `last_render_time` initializes to 16ms (mod.rs:712-713), so the first `FpsLimit` frame would panic the interactive loop the day anyone wires the variant up. Fields `timer` and `target_duration_between_renders` serve only the dead arm; `last_render_time` is written every frame (mod.rs:860, 871) but read only by the dead arm.
Why it matters: §5 "no dead code"; §9 interactive paths must not panic. Dead code that panics on revival is worse than plain dead code.
Fix: Delete the `OnRequest`/`FpsLimit` arms plus `timer`/`target_duration_between_renders`/`last_render_time` (or, if FPS-limiting is on the named trajectory, implement the arm with `saturating_sub`, add the setter, and test it in the same commit).
Effort: S
### R4. Retired picker title/hint/chip surfaces left half-removed: dead geometry fields allocated per rebuild, chip constraint still steering layout
Severity: P2 | Category: dead-code | Confidence: high
Files: src/application/color_picker/geometry.rs:19, 23, 114, src/application/color_picker/compute_sizing.rs:87-88, src/application/app/color_picker_flow/rebuild.rs:47-52 (producer), src/application/app/color_picker_flow/geometry.rs:140, src/application/app/color_picker_flow/commit.rs:63 ("chip row has been retired"), src/application/color_picker_overlay/picker_glyph_areas/compute.rs:35-39 (only 5 sections built)
Evidence: `ColorPickerOverlayGeometry.target_label`, `.preview_hex`, and `.selection_hint` are produced on every picker rebuild — `standalone_selection_hint(&doc.selection)` allocates a `String` per rebuild (rebuild.rs:47-52) and `preview_hex: hsv_to_hex(...)` allocates another (geometry.rs:140) — but no overlay builder reads any of them (the sections built are hue_ring/sat_bar/val_bar/preview/hex only; the hex section recomputes its own `hsv_to_hex`, sections/hex.rs:29). The title/hint sections that consumed them were retired from `color_picker.json`. Meanwhile `compute_sizing.rs:87-88` still clamps font size by a phantom chip row:
```rust
let chip_width_in_fonts: f32 = 32.0;
let max_font_for_w = (screen_w / (wheel_side_in_fonts + 2.0).max(chip_width_in_fonts)).max(1.0);
```
On a 390px-wide phone viewport this caps the picker font at ~12px for a chip row that no longer exists. `selection_hint`'s own doc still promises "Without this, a Standalone wheel commit ... gives the user no signal" (geometry.rs:105-114) — a promise nothing renders.
Why it matters: §5 (no half-features, delete rather than deprecate, §10). Per-rebuild allocations for unread fields violate §B1; the chip clamp silently distorts mobile sizing.
Fix: Remove the three fields, their producers, and the chip-width clamp (or re-introduce the hint section if the promise is wanted — but then render it). Sweep the "chip" vocabulary out of the picker/renderer doc comments (see R14).
Effort: M

### R5. Renderer hitbox maps re-implement hit-testing that the canvas trees + `Scene::component_at`/BVH already provide
Severity: P2 | Category: duplication | Confidence: high
Files: src/application/renderer/hit.rs:17-29 (aabb_contains, find_first_aabb_hit), hit.rs:61-147 (hit_test_edge_label, hit_test_any_edge_label, hit_test_portal, hit_test_portal_text), src/application/renderer/mod.rs:371-394 (the three hitbox maps), src/application/app/scene_rebuild.rs:616, 693-699 (producers; comment literally says "legacy `Renderer::hit_test_portal`"), lib/baumhard/src/gfx_structs/scene.rs:257-286 (Scene::component_at), lib/baumhard/src/gfx_structs/shape.rs:117-136 (contains_local)
Evidence: portal glyphs and connection labels already live in canvas trees registered under `CanvasRole::Portals` / `CanvasRole::ConnectionLabels` (mod.rs:377-380 doc: "Portal glyph buffers themselves flow through canvas_scene_buffers via the tree pipeline"), and baumhard's `Scene::component_at` + `Tree::descendant_at` provide memoized-AABB + BVH hit resolution over exactly those trees. The renderer nonetheless maintains three parallel flat `FxHashMap<_, (Vec2, Vec2)>` indexes, populated wholesale by the tree builders through `set_portal_icon_hitboxes`/`set_portal_text_hitboxes`/`set_connection_label_hitboxes` (hit.rs:87-117) and scanned linearly by `find_first_aabb_hit`. Document-side `hit_test.rs` (hit_test/hit_test_target/hit_test_edge/rect_select) covers nodes/sections/edges via the tree BVH — no body-level overlap with the renderer's functions, so the duplication is renderer-vs-baumhard, not renderer-vs-document. `scene_host.rs:421-425` acknowledges the split ("unifying those two paths is deferred work").
Why it matters: §1 "reach for the existing seam" — the hitbox maps are a second spatial index over data that already has one, kept in sync by hand at every portal/label rebuild. Two sync points, two iteration orders, two tolerance regimes.
Fix: When portal/label hit-testing next changes, resolve clicks through `app_scene.canvas_scene()` + `component_at` (with a role filter and per-element identity read from the tree, e.g. channel → (EdgeKey, endpoint) mapping owned by the tree builder), then delete the three maps + setters. Until then, at minimum note the linear-scan/nondeterminism caveat (R11).
Effort: L

### R6. `mindmap_buffers` keyed by stringified `usize` — String allocation per element per rebuild and per drag-patch
Severity: P2 | Category: performance | Confidence: high
Files: src/application/renderer/mod.rs:344-355, src/application/renderer/tree_buffers.rs:43, 90, 100, 139
Evidence:
```rust
mindmap_buffers: FxHashMap<String, Vec<MindMapTextBuffer>>,   // mod.rs:355
self.mindmap_buffers.entry(unique_id.to_string())...          // tree_buffers.rs:43 (every element, every rebuild)
let key = unique_id.to_string();                              // tree_buffers.rs:139 (every patch, every drained drag frame)
```
Every key in the map is `unique_id.to_string()` of a `usize`; nothing ever looks up by a non-numeric string. The field doc's justification ("stringified for use as a FxHashMap key alongside the edit / undo paths' Dewey-decimal addressing", mod.rs:345-347) does not hold — no Dewey string ever reaches this map. `patch_drag_positions` allocates one `String` per moved node per drained drag frame; `rebuild_buffers_from_tree` allocates one per element per rebuild; hashing a `String` also costs more than hashing a `usize` in the per-frame `values()` iteration capacity sum (render.rs:320).
Why it matters: §B1 "no new allocations in hot loops"; the drag-drain path is the designed hot path (`patch_drag_positions` exists precisely to be cheap).
Fix: `FxHashMap<usize, Vec<MindMapTextBuffer>>`; delete the stale doc justification. Callers already hold the `usize`.
Effort: S

### R7. Node/section resize drags re-shape the entire arena on every drained frame
Severity: P2 | Category: performance | Confidence: med
Files: src/application/app/throttled_interaction/section_resize.rs:133, node_resize.rs:108 (callers, outside scope but on the frame path), src/application/renderer/tree_buffers.rs:20-49 (the full rebuild), tree_buffers.rs:76-107 (the keyed alternative)
Evidence: the resize drain arms call `renderer.rebuild_buffers_from_tree(&tree.tree)` — a full-arena walk that re-shapes every text buffer in the document — once per drained frame while a resize handle is dragged. The move-drag path shows the intended shape: `patch_drag_positions` + `rebuild_node_backgrounds_from_tree` (moving_node.rs:100-101), no shaping at all. Only the resized container + its sections change bounds during a resize; the renderer already exposes the keyed `reshape_buffer_for(arena_id, tree)` for exactly this.
Why it matters: §B1/§4 — on a large map, a section-resize drag on a phone pays O(all nodes) cosmic-text shaping per drain; the adaptive throttle then degrades cadence to hide it, i.e. fidelity is spent where a keyed reshape would spend nothing.
Fix: In the resize drains, reshape only the affected container + its section elements (loop `reshape_buffer_for` over the container's arena children, or add a `reshape_subtree_for` beside it).
Effort: M

### R8. Layout-phase section builders and dynamic-phase context duplicate the per-cell color/hover/font logic
Severity: P2 | Category: duplication | Confidence: med
Files: src/application/color_picker_overlay/picker_glyph_areas/sections/hue_ring.rs:31-43, sat_bar.rs:35-58, val_bar.rs:35-58, preview.rs:30-41 vs src/application/color_picker_overlay/picker_glyph_areas/dynamic_context.rs:265-288 (scale_for), 340-396 (field)
Evidence: the "hovered → highlight_hovered; selected → highlight_selected; else base; bottom arm pins `arm_bottom_font()`; preview pins Tibetan; hover bumps scale" decision tree exists twice — once spread over the four section builders, once in `PickerDynamicContext::{scale_for, field}`. Example pair: sat_bar.rs:43-50 vs dynamic_context.rs:352-363 are the same three-way branch. The two copies are pinned against drift by the round-trip tests (`picker_dynamic_mutator_composes_on_layout_built_tree`, `picker_mutator_round_trips_to_fresh_build`) and by explicit mirror comments ("Mirror that exactly", dynamic_context.rs:302-308), which is why this is P2 not P1 — but the mirror is maintained by test-failure, not by construction.
Why it matters: §5 "identical logic copy/pasted ... the answer is never to copy it". Every new picker cell state (e.g. a disabled state, a keyboard-focus ring) must be implemented twice and round-trip-tested.
Fix: Extract a shared `cell_visual(section, index, geometry, base_rgb) -> (Color, scale_factor, Option<AppFont>)` used by both the section builders and `PickerDynamicContext`; the builders keep position/bounds, the dynamic context keeps its precomputed tables as the `base_rgb` source.
Effort: M

### R9. Console overlay hand-rolls the channel-scheme + full-assign mutator that the picker already gets from `baumhard::mutator_builder`
Severity: P2 | Category: duplication | Confidence: med
Files: src/application/renderer/console_pass.rs:43-50 (channel constants), 393-405 (tree builder loop), 424-441 (mutator builder loop), 451-458 (signature) vs src/application/color_picker_overlay/picker_glyph_areas/trees.rs:31-43, 57-65 (spec-driven equivalents), src/application/scene_host.rs:96-102 (hash_canvas_signature)
Evidence: the console and picker overlay passes share the whole §B2 shape — stable channel bands, areas-fn as single source, tree builder (append `GfxElement::new_area_non_indexed_with_id(area, channel, channel)` per area — console_pass.rs:399-403 and trees.rs:37-41 are line-for-line the same loop), full-assign `DeltaGlyphArea` mutator per area, structural signature. The picker drives all of it from a declarative `MutatorNode` spec + `mutator_builder::build`; the console re-implements each piece by hand, including its own `DefaultHasher` incantation at console_pass.rs:451-458 that duplicates `scene_host::hash_canvas_signature` (the helper whose doc says "one DefaultHasher incantation instead of four").
Why it matters: §2 "unify the shapes"; §6. Three overlay-ish passes were the quest — the borders pass legitimately differs (flat-scene, transitional, other agent's scope), but console vs picker is the same abstraction implemented twice at different maturity levels.
Fix: Short term: `console_overlay_signature` → `hash_canvas_signature(&(layout.scrollback_rows, layout.completion_rows))`; share one `fn tree_from_areas(areas) -> Tree` + `fn full_assign_mutator_from_areas(areas) -> MutatorTree` between both passes. Longer term: express the console's channel layout as a (hardcoded) `MutatorNode` spec and route through `mutator_builder` like the picker.
Effort: M
### R10. Picker JSON/Rust drift panics are reachable from interactive paths — and the JSON is slated to become user-authored
Severity: P2 | Category: error-handling | Confidence: high
Files: src/application/color_picker/glyph_tables.rs:51-63 (picker_channel panic + per-call `section.to_string()` allocation), src/application/color_picker_overlay/picker_glyph_areas/areas.rs:49-58 (from_name panic), 77-87 (push expect), 96-111 (area panic), src/application/widgets/color_picker_widget.rs:144-149 (load_spec expect), sections/hue_ring.rs:41 / sat_bar.rs:52-54 / val_bar.rs:51-57 (spec-count-trusting indexing into `layout.*_positions[i]` and arm arrays), src/application/widgets/color_picker.json:2 ("First step toward user-authored widgets")
Evidence: `picker_channel` panics on unknown `(section, index)`; `PickerSection::from_name`/`PickerAreas::{push, area}` panic on drift; `load_spec` `expect`s on malformed JSON. All fire lazily behind `OnceLock`s at the first picker open / hover — squarely inside `Application::run`'s post-first-frame window, not the sanctioned startup window (§9). Today the JSON ships via `include_str!` and every invariant is pinned by the `spec_*` tests, so drift is a developer error caught by `./test.sh` — but the file's own header names user-authored widget specs as the trajectory, at which point every one of these panics becomes user-input-triggered. Secondary: `picker_channel` allocates a `String` per lookup (`map.get(&(section.to_string(), index))`, glyph_tables.rs:61) and is called once per cell (~58×) per layout-phase compute.
Why it matters: §9 "interactive paths must not panic" is absolute; §7 says a hard-to-support use case (user JSON) is a constraint on the design.
Fix: Validate the spec once inside the startup window (e.g. a `widgets::validate_specs()` called next to `fonts::init()`, where `expect` is sanctioned) so first-use panics move to startup; keep the internal lookups as `debug_assert!` + warn-and-degrade arms (the pattern dynamic_context.rs:318-331 already demonstrates). Key the channel cache by `(&'static str, usize)` or per-section `Vec<usize>` to kill the per-lookup allocation.
Effort: M

### R11. `find_first_aabb_hit` returns a hash-iteration-order winner when hitboxes overlap
Severity: P3 | Category: correctness | Confidence: med
Files: src/application/renderer/hit.rs:21-29, 77-79, 135-147
Evidence: "First key in `map` whose AABB contains `pos`; linear scan" — over a `FxHashMap`, so "first" is arbitrary and can differ across runs/insertions. Portal icon-vs-text overlap is argued away in the doc (hit.rs:140-144), but two *different* portals' icons, or two edge labels on crossing edges, can overlap; the click target then depends on hash order.
Why it matters: nondeterministic input routing is untestable and produces "sometimes it selects the other label" reports. §T1 platform-shared logic must behave identically across runs.
Fix: Deterministic tie-break (e.g. smallest-area hitbox wins, or ordered iteration over a sorted key list); or fold into R5's `component_at` migration, which already has layer-order determinism.
Effort: S

### R12. Dead renderer surface: never-emitted decree variants, never-written buffer field, uncalled camera fit, production-unused hue quantizer, false re-export justification
Severity: P2 | Category: dead-code | Confidence: high
Files: src/application/common.rs:72, decree.rs:45 (ReinitAdapter — zero emitters), decree.rs:42-44 (StopRender — zero emitters; grep shows only the handler), src/application/renderer/mod.rs:396-398, 740 + render.rs:363, 369 (console_overlay_buffers: declared, initialized, chained into the palette pass — never pushed/cleared anywhere since the console migrated to the overlay-tree path), src/application/renderer/hit.rs:32-55 (fit_camera_to_scene — zero callers; fit_camera_to_tree is the live one), src/application/color_picker/glyph_tables.rs:171-175 (degrees_to_hue_slot — only consumers are its own tests), src/application/renderer/mod.rs:52-62 (comment claims "external callers (the app crate threads ConsoleFrameLayout through the rebuild path)" — grep finds no app-crate consumer of ConsoleFrameLayout/compute_console_frame_layout/build_console_border_strings outside renderer/)
Why it matters: §5 "no dead code"; every merged state is one we would ship. The false `#[allow(unused_imports)]` justification actively misleads the next reader; the always-empty `console_overlay_buffers` costs a per-frame `.len()`/chain and implies a render path that no longer exists (overlay_dispatch.rs:30 doc still claims the pass "draws console_overlay_buffers").
Fix: Delete `ReinitAdapter`; delete or wire `StopRender` (CONCEPTS mentions it — if the idle path should use it, wire it; otherwise drop it and fix CONCEPTS); remove `console_overlay_buffers` and its render.rs chain links; remove `fit_camera_to_scene`; move `degrees_to_hue_slot` into the test module or add its production consumer; correct the mod.rs re-export comment to "tests + measure consumers" and drop the unused names from it.
Effort: M

### R13. Decree audit (quest 4): sound boundary overall, but WASM wheel-zoom bypasses the Action funnel with a hardcoded factor
Severity: P3 | Category: convention | Confidence: high
Files: src/application/renderer/decree.rs:14-81 (sole consumer), emitters: src/application/app/event_cursor_moved.rs:142, 571 (CameraPan — sanctioned per-frame carve-out), src/application/app/dispatch/cross_dispatch/camera.rs:47, 64, 105 (Action arms), run_native.rs:371, 406, run_native_init.rs:42, 158, run_wasm/mod.rs:645, 749, run_wasm/event_resized.rs:17 (lifecycle), src/application/app/run_wasm/event_mouse_wheel.rs:24, 36-40
Evidence: the decree vocabulary is a renderer-mutation entry point, not a second dispatch system: discrete effects arrive *from* Action arms (`ToggleFps` → `set_fps_display` → `SetFpsDisplay`; zoom Actions → cross_dispatch/camera.rs), continuous per-frame deltas (`CameraPan`) are the CODE_CONVENTIONS §3 carve-out, and no decree body duplicates an Action body. Two blemishes: (a) `handle_mouse_wheel` on WASM computes `factor = 1.1` inline and emits `CameraZoom` directly — the native wheel resolves through `action_for_gesture(WheelUp/WheelDown)`, so WASM users cannot rebind/disable wheel zoom and the factor is duplicated; (b) mod.rs:32-33 calls decree.rs "the `RenderDecree` queue" — it is synchronous dispatch, nothing queues.
Why it matters: §3 single dispatch funnel; §4 cross-platform first class (WASM input must not be a hardcoded fork).
Fix: Route WASM wheel through the same gesture-name → `action_for_gesture` lookup as native (the Action arm already exists in cross_dispatch/camera.rs); reword the "queue" doc.
Effort: S

### R14. Stale / lying doc comments across the scope (13 distinct sites)
Severity: P3 | Category: docs | Confidence: high
Files+Evidence:
- src/application/renderer/mod.rs:246-249: claims `shape_id` is stored "as an `f32` holding the `u32` bit pattern via `f32::from_bits`" — actual code is `shape_id as f32` + WGSL `u32(round(id))` (render.rs:49, mod.rs:143). The described encoding would render nothing.
- src/application/widgets/color_picker_widget.rs:31-40: "10 glyphs" ×4 arm docs — actual arrays are 8 (JSON + spec_loads test assert 8).
- src/application/color_picker/glyph_tables.rs:44 and color_picker_overlay/picker_glyph_areas/mod.rs:10-12: section lists still include "title" and "hint" — retired; JSON has 5 sections.
- Chip remnants: color_picker/mod.rs:9-11 ("row of theme-variable quick-pick chips", "Tab cycles chips"), widgets/mod.rs:6 ("chip list"), renderer/overlay_dispatch.rs:96, 126, 152, renderer/color_picker.rs:40, 59, color_picker_overlay/trees.rs:54, color_picker/geometry.rs:95, tests/layout.rs:137 — plus CONCEPTS.md §6 picker entry. `commit.rs:63` states the chip row is retired.
- src/application/color_picker/geometry.rs:5, 12: "mirrors `PaletteOverlayGeometry`" — no such type exists (it is `ConsoleOverlayGeometry`).
- src/application/common.rs:92-94: `FpsDisplayMode::Debug` promises "per-stage timing breakdown (event drain, scene build, GPU submit)" — actual behavior is a rolling-average FPS integer (mod.rs:1022-1031).
- src/application/renderer/overlay_dispatch.rs:99-104: "the planned `MutatorTree`-based hover path will mutate only changed cell colors" — that path exists 50 lines below (`apply_color_picker_overlay_dynamic_mutator`).
- color_picker_overlay/tests/build_shape.rs:4-5, tests/mod.rs:8, tests/mutator_round_trip.rs:66-68: reference "GlyphArea/GlyphModel pairing" / "paired GlyphModel children" — the tree is flat GlyphArea leaves and the test itself asserts that.
- src/application/widgets/color_picker.json:49: "See src/application/mutator_builder/ for the walker" — it lives in baumhard.
- src/application/color_picker/tests/layout.rs:259-260: "at the new 1.5× font scale" — spec is 2.3.
- Wrong convention citations: console_pass.rs:211, dynamic_context.rs:319-321, renderer/tests.rs:424 cite "§7" for interactive-paths-never-abort; that rule is §9.
- src/application/renderer/mod.rs:32-33: "decree queue" (see R13).
Why it matters: §8 "a doc comment that lies about its function is worse than no doc comment"; the shape_id one describes a load-bearing GPU encoding incorrectly.
Fix: one doc-sweep commit; update CONCEPTS.md §6 picker entry in the same pass.
Effort: S

### R15. British spellings throughout scope comments (project mandates American English)
Severity: P3 | Category: convention | Confidence: high
Files: 59 grep hits across the scope, e.g. renderer/tree_buffers.rs:65 ("colour"), renderer/hit.rs:149 ("centred"), renderer/render.rs:202 ("rasterised"), renderer/scene_buffers.rs:52, 68, color_picker/state.rs:33, 149-151 ("centre"), color_picker/targets.rs:226-232, color_picker/compute_positions.rs:6, 59, color_picker_overlay/tests/mod.rs:4 ("honour"), tests/build_shape.rs:14-20, renderer/tests.rs:422 ("behaviour"), frame_throttle.rs:33 ("behaviour"), plus assertion-message strings (color_picker/tests/layout.rs:253 "from centre", targets tests "colour").
Why it matters: CLAUDE.md §6 "Use American English for consistency". §2 of the review charter: cosmetic is not skippable.
Fix: mechanical sweep (colour→color, centre→center, behaviour→behavior, honour→honor, rasterised→rasterized, quantise→quantize) over comments, docs, and test-assertion strings in scope; baumhard files (e.g. font/color.rs module doc "quantisation") have the same issue outside this scope.
Effort: S

### R16. frame_throttle test suite duplicates six scenarios under two naming schemes
Severity: P3 | Category: testing | Confidence: high
Files: src/application/frame_throttle.rs:156-349 vs 351-513
Evidence: the "§T1 comprehensive coverage" banner block re-tests already-covered behavior nearly one-for-one: `reset_returns_to_fresh_state` (295) vs `test_reset_returns_to_fresh_state` (453); `load_drop_decays_n_toward_one` (197) vs `test_recovery_lowers_n` (424); `sustained_over_budget_raises_n` (174) vs `test_over_budget_raises_n` (378); `very_heavy_load_caps_at_max_n` (186) vs `test_n_clamped_at_max_n` (410); `decay_has_hysteresis_around_budget` (223) vs `test_hysteresis_prevents_oscillation` (477); `healthy_load_drains_every_frame` (163) vs `test_under_budget_keeps_n_at_one` (366). The two halves also split on the §T3 `test_<topic>_<case>` naming rule — a split that recurs across the scope (renderer/tests.rs mixes `test_console_*` with `console_mutator_round_trips…`/`clamp_surface_size_*`; all color_picker/color_picker_overlay/scene_host tests omit the prefix).
Why it matters: TEST_CONVENTIONS §T3 naming; duplicated tests dilute the suite and make behavioral drift reviews slower (§T12 wants aggressive tests, not repeated ones).
Fix: merge each pair keeping the stricter body; pick one naming scheme (the documented `test_` prefix) and apply it in the merged file; treat the prefix rule as advisory-to-update if the unprefixed style is now preferred (then amend §T3 instead — conventions change in response to code, but explicitly).
Effort: S

### R17. Minor per-event allocation and API nits
Severity: P3 | Category: performance | Confidence: high
Files: src/application/renderer/hit.rs:87-117, src/application/renderer/render.rs:320-336, 363-383, src/application/scene_host.rs:228-234, lib/baumhard/src/gfx_structs/scene.rs:231-234
Evidence: (a) `set_*_hitboxes` take `std::collections::HashMap` and re-collect into `FxHashMap` per portal/label rebuild — take `FxHashMap` (or an iterator) and move it. (b) `prepare_text_for_pass` allocates two `Vec<TextArea>` every frame; capacity is precomputed (good) but the vectors could be `self`-owned scratch (`Vec::clear` + repopulate) — TextArea borrows buffers, so scratch must be local per call; acceptable, note only. (c) `AppScene::canvas_ids_in_layer_order`/`overlay_ids_in_layer_order` clone a `Vec<SceneTreeId>` per rebuild on top of `Scene::ids_in_layer_order`'s own clone — one clone could be spared by returning `&[SceneTreeId]` from a cached sort. All are rebuild-frequency, not per-frame; listed for the §B1 ledger, not urgency.
Fix: as above, opportunistically.
Effort: S

### R18. `patch_drag_positions` silently collapses halo offsets (documented latent bug)
Severity: P3 | Category: correctness | Confidence: high
Files: src/application/renderer/tree_buffers.rs:118-136 (the "Halo limitation" doc), 137-146
Evidence: the patch path overwrites every buffer's `pos` with `new_pos`, which would collapse the 8 halo stamps onto the main glyph for any mindmap element that ever sets `area.outline`. Today no mindmap element does (halos are picker-only), and the limitation + fix sketch are documented in place.
Why it matters: CLAUDE.md §5 discourages leaving documented landmines; the moment outline halos become a node style (a plausible creative-tool feature per §7), drag visually breaks with no compiler signal.
Fix: store the per-buffer `(dx, dy)` emission offset on `MindMapTextBuffer` (or in the map value) and patch `pos = new_pos + offset` — removes the caveat for one small field.
Effort: S

### R19. Renderer `tree_walker.rs` name collides with baumhard's mutator walker (different semantics)
Severity: P3 | Category: api-design | Confidence: high
Files: src/application/renderer/tree_walker.rs (tree → shaped-buffer projection) vs lib/baumhard/src/gfx_structs/tree_walker.rs (MutatorTree application via walk_tree_from / align_child_walks)
Evidence: two modules named `tree_walker` with unrelated jobs; cross-references amplify the confusion — e.g. glyph_tables.rs:31 cites "Baumhard's `align_child_walks` ... see `tree_walker.rs:226`" in a crate whose own `tree_walker.rs` is 206 lines long. Substantively the renderer walker is clean §1-wise: it consumes baumhard bridges (`RegionFamilies::resolve`, `rich_text_spans_from_regions`, `buffer::create`), iterates via `descendants(&arena)` per §B4, and does not re-implement zoom-gating or culling (those go through `ZoomVisibility::contains` + `Camera2D::is_visible` at render time). The collision is purely a naming/navigation hazard.
Fix: rename the renderer module (e.g. `buffer_projection.rs` or `shape_walk.rs`) and qualify the baumhard cross-references with the crate path.
Effort: S

### R20. WGSL↔Rust shape-constant lock-step is comment-enforced only
Severity: P3 | Category: testing | Confidence: high
Files: src/application/renderer/mod.rs:115-168 (RECT_SHADER_WGSL), lib/baumhard/src/gfx_structs/shape.rs:49-55, 117-136
Evidence: verified the two sides agree today — `SHAPE_RECT=0u`/`SHAPE_ELLIPSE=1u` match `SHAPE_ID_RECTANGLE=0`/`SHAPE_ID_ELLIPSE=1`, and boundary semantics match (`contains_local` ellipse `nx²+ny² <= 1.0` inclusive vs shader `if (d > 1.0) discard` inclusive; rectangle closed-interval vs full-quad). But the "must stay in lock-step" contract (shape.rs:57-62, mod.rs:104-106) has no test: a renamed WGSL constant or a new enum variant without a shader case degrades silently to rectangle fill.
Fix: a plain-string test in renderer/tests.rs asserting `RECT_SHADER_WGSL.contains("const SHAPE_ELLIPSE: u32 = 1u")` etc., iterating the `NodeShape` variants — pure text, §T8-compatible.
Effort: S

## Checked and CLEAN

- FONT_SYSTEM discipline (quest 2): frame path (`prepare_text_for_pass`, FPS/mode-status reshapes) consistently uses `try_write` + skip-frame; rebuild paths consistently use `acquire_font_system_write` (bounded 5s try-loop, panic-with-site — a deliberate deadlock-diagnostic per its baumhard doc); the 9× halo shaping runs under a single acquisition threaded through the walker; both glyphon `prepare` calls share one guard; no lock held across GPU submit.
- HSV↔RGB single-source (quest 7): all conversions route to `baumhard::util::color_conversion` (hsv_to_rgb/rgb_to_hsv/hsv_to_hex/hex_to_hsv_safe); `font/color.rs` is quantization-only and delegates to the same arithmetic; no app-crate reimplementation found.
- Shader↔hit-test shape semantics (quest 10): `hit_test_picker`'s outer gate routes through `NodeShape::Ellipse.contains_local`; shader and Rust agree on ids and inclusive boundaries (see R20 for the missing pin).
- Buffer-store leak audit (quest 1): every keyed store is clear-then-rebuild (`mindmap_buffers`, `border_buffers`, `connection_label_buffers`+hitboxes, `edge_handle_buffers`) or wholesale-replace (portal hitbox setters); deleted nodes/edges cannot leave stale entries; rect vertex buffer grows-doubling and is reused; `main_rect_vertices`/`console_rect_vertices` are cleared, not reallocated.
- Frame-path buffer reuse: selection-rect shape cache skips 4 shapings per drag tick on cache hit; FPS/mode-status overlays re-shape only on value change with lock-contention-safe cache-key advance (the pre-fix bug is documented and fixed).
- `MutationFrequencyThrottle` (quest 9): math correct (moving average over drained frames, 30% hysteresis, n clamped [1, MAX_N=8], reset-on-drag-end); all seven ThrottledDrag variants plus `color_picker_hover` own one uniformly via `with_default_budget()` + the trait `throttle()` accessor; test coverage heavy (modulo the duplication in R16).
- §T8 posture (quest 11): renderer/tests.rs is pure layout math + ring arithmetic + clamp logic — no wgpu, no device; backdrop-vs-border alignment, scroll-window clamps, cull-rect composition (spatial AND zoom), FrameIntervalRing sum invariant, and §B2 console round-trips are all pinned.
- scene_host §B2 dispatch (quest 8): rebuild-or-mutate decision is single-sourced (`canvas_dispatch`/`overlay_dispatch`); signatures cleared on unregister; per-role signature definitions are deliberately caller-owned; register/unregister role slots can't leak slab entries (tests pin it). Only the enum-mirroring and hasher-bypass nits (R9) stand.
- Interactive-path degrade patterns: `console_overlay_areas` warn-and-skip on slot invariant violation (with tests pinning the degrade), `render()` early-returns on surface loss and prepare failure, `prewarm` catch_unwind is documented and startup-scoped, undo/hit paths in scope bounds-check before indexing.
- No `TODO`/`FIXME`/`HACK` markers anywhere in scope.
- `NodeBackgroundRect`/`MindMapTextBuffer` culling: `visible_at` composes spatial AND zoom-window; per-frame, allocation-free; tests cover both bounds and the unbounded default.
