# Mandala Codebase Refactor — Rainforest in Equilibrium

## Context

A 10-agent (opus) read-only audit of the Mandala workspace (~118k LoC of Rust
across `lib/baumhard`, `src/application`, `crates/maptool`, `lib/mandala_derive`)
surfaced ~280 concrete findings. The codebase is already disciplined in many
ways (no `// TODO/FIXME/HACK` markers in `src/` or `lib/baumhard/src/`, the
mutation-not-rebuild discipline is honoured, no live-wgpu tests, no mocks),
but several patterns have accreted that violate the workspace's own stated
goals — `CLAUDE.md`, `CODE_CONVENTIONS.md`, `lib/baumhard/CONVENTIONS.md`,
`TEST_CONVENTIONS.md`. This plan turns the audit findings into a sequenced,
multi-session refactor whose end state is the rainforest the user named:
nothing in excess, nothing lacking, every component pulling its weight.

The plan is structured so each batch is independently shippable, in dependency
order — early batches unblock later ones. Stop after any batch and the codebase
is strictly better than before.

## Cross-cutting themes (what kept showing up)

1. **Three Color types in two crates, one of them mathematically broken.**
   `baumhard::util::color::Color::to_float` (lib/baumhard/src/util/color.rs:250)
   uses `u8` integer division — every non-saturated channel collapses to 0.0,
   the doc-comment admits this. There are also two open-coded `[r,g,b,a]/255.0`
   converters in the renderer (`console_pass.rs:112`, `render.rs:126`) plus
   `cosmic_to_rgba` (`color_picker_overlay/picker_glyph_areas/dynamic_context.rs:301`)
   and `rgb_to_cosmic_color` (`color_picker_overlay/color.rs:14`) that
   reimplement quantisation already done by `convert_f32_to_u8`. One canonical
   `Color` with `from_rgb_f32` / `to_rgba_f32` kills five sites.

2. **Wrappers that exist are bypassed; wrappers that should exist don't.**
   `cosmic_text` is wrapped in `lib/baumhard/src/font/` but the renderer
   constructs `cosmic_text::Buffer/Metrics/Shaping/Align/Color` directly at
   ~12 sites (`renderer/borders.rs`, `scene_buffers.rs`, `tree_walker.rs`,
   `console_pass.rs`, `render.rs`, `mod.rs`). `wgpu` is wrapped by `Renderer`
   but `app/run_native_init.rs` and `app/run_wasm/mod.rs` reach into
   `wgpu::Instance` directly. `winit` has **no wrapper at all** — `Key`,
   `Modifiers`, `MouseButton`, `KeyEvent` leak into keybinds, label/text
   editors, every event handler (15+ sites). `log` has **no wrapper** — 22+
   files import the `log` crate directly. `serde_json` has **no wrapper** —
   four loaders bind to it directly.

3. **Half-features shipped as scaffolding.**
   - Color picker `title`/`hint` sections: built every frame, never dispatched
     (`color_picker_overlay/picker_glyph_areas/{title,hint}.rs`,
     `compute.rs:35-39` ignores them).
   - `Renderer` carries six dead fields (`instance`, `adapter`,
     `surface_capabilities`, `texture_format`, `shaders`, `render_pipeline`)
     and a `default_pipeline` that's never bound (`renderer/mod.rs:250-266`).
   - `tree_buffers.rs` keyed-rebuild fast path is unreachable —
     `dirty_node_ids = None` at every call site (`scene_buffers.rs:43-86`).
   - `glyph_model_from_picker_area` builds a `GlyphModel` mirror that the
     renderer explicitly ignores (`color_picker_overlay/glyph_model.rs`).
   - `Instruction::RotateWhile` has an empty arm (`tree_walker.rs:159`).
   - Maptool ships `unique_node_ids`, `migrate_one_node_legacy`, `show_node`
     as one-line wrappers over functionality that exists elsewhere.

4. **God-functions and god-structs.**
   - `handle_cursor_moved` — 540 lines, 7 throttled-drag arms each with
     duplicated `Pending`-promotion shape (`event_cursor_moved.rs:27-545`).
   - `Renderer::new` — 178 lines (`renderer/mod.rs:529-706`); `Renderer::render`
     — 295 lines (`renderer/render.rs:84-378`).
   - `KeybindConfig::resolve` — 375 lines, hand-maintained mirror of the
     `Action` enum (`keybinds/config.rs:420-795`); a derive macro fits.
   - `Predicate::test` — 300-line nested match (`gfx_structs/predicate.rs:176-485`).
   - `execute_font` — 185 lines, parallel to a trait dispatcher that already
     handles every other axis (`console/commands/font.rs:174-358`).
   - `MindMapDocument` — 12 public fields; `InputHandlerContext` — 21 fields,
     `DragState::Pending` — 8 `Option`s.

5. **Tests that test the type system or the framework.**
   - `lib/baumhard/src/util/tests/primes_test.rs` — 1242 lines of
     `assert!(is_prime(N))` for every prime under 10000 (testing
     `Sieve of Eratosthenes` itself).
   - `lib/baumhard/src/gfx_structs/camera.rs:216-307` ships a duplicate
     inline `mod tests` next to the canonical `tests/camera_tests.rs`
     (violates `TEST_CONVENTIONS §T2.2`).
   - 35+ specific tautological tests across `element_tests.rs`,
     `zoom_visibility_tests.rs`, `console/tests/state.rs` and more.

6. **Documentation drift.**
   - 0-byte workspace `README.md` (the file GitHub renders to first-time
     visitors).
   - `TODO.md` is ~75% strikethrough "Shipped" entries.
   - `WASM_CONVERGENCE.md` Track B/C sections are postmortems for finished
     work plus commit-hash lists that belong in `git log`.
   - `CONCEPTS.md` is 140 KB; ~45 KB is removable (the §9 Glossary duplicates
     the table of contents; §8 Vision duplicates per-concept Vision blurbs;
     "Status" subsections inside concept entries are embedded changelogs).
   - `lib/baumhard/todo.txt` is dev-diary prose ("Thank God I wrote this").

7. **Hot-path allocations.**
   - `EdgeKey::new` allocates 3 `String`s per cache lookup
     (`mindmap/scene_cache.rs:56-62`).
   - `mindmap_buffers` is keyed by `unique_id.to_string()` per element per
     frame (`renderer/tree_buffers.rs:42-44`).
   - `main_text_areas` builds a fresh `Vec<TextArea>` from six chained
     iterators every frame (`renderer/render.rs:218-243`).
   - `rebuild_selection_rect_overlay` shapes 4 fresh `Buffer`s per drag tick
     (`renderer/scene_buffers.rs:230-289`).
   - `BorderGlyphSet::side_patterns` allocates `Vec<String>` of one-grapheme
     strings per visible-framed-node per scene-build (`mindmap/border.rs:597`).
   - `display_text` always allocates even for single-section nodes
     (`mindmap/model/node.rs:192-201`).

## How this plan is sequenced

The 8 batches below are roughly independent; cross-batch dependencies are
called out in the batch headers. Batches are sized so each is completable
inside one or two sessions of focused work plus a `./test.sh` cycle. Inside
each batch, items are listed by impact-per-effort.

After every batch: run `./test.sh`, run `./test.sh --lint`, manually open one
mindmap (`maps/testament.mindmap.json`) on native AND WASM and exercise the
golden path (open, drag, zoom, click an edge label, open the console, open
the color picker, undo). The renderer and dispatch layer are the ones most
likely to regress; the test suite cannot catch UI regressions.

The plan file `REFACTOR_PLAN.md` lives at the repo root so that successor
sessions can resume by reading it. Tick boxes are flipped as each item lands.

---

## Batch 1 — Deletions (low-risk, immediate signal-to-noise win)

Pure removals; zero new abstractions. Each item is a `git rm` + a few callsite
deletions. Self-contained.

### 1.1 Delete dead documentation
- [ ] `TODO.md`: delete the entire "Shipped on this branch" + per-track
      strikethrough lists (lines 8–137); replace with a 5-line pointer to
      git history. Keep only the one outstanding thread (filesystem-on-WASM)
      as a 4-line bullet.
- [ ] `WASM_CONVERGENCE.md`: collapse Track B SHIPPED (lines 140–219) and
      Track C SHIPPED (lines 220–310) to one paragraph each. Remove commit
      hashes (`b60569a`, `37c2897`, `1fd2eeb`) — git log owns those.
- [ ] `CONCEPTS.md §9 Glossary` (lines 2792–3087, ~16 KB): delete entirely.
      The headings + table of contents in §1 already serve as an index.
- [ ] `CONCEPTS.md §8 Named trajectory — vision` (lines 2683–2789, ~6 KB):
      delete; the per-concept `**Vision.**` blurbs cover the same ground
      with better local context (the doc itself admits the duplication at
      line 2697).
- [ ] `CONCEPTS.md` "Status" / "shipped-tier" subsections inside concept
      entries (e.g. `MindSection` lines 1039–1102, `SelectionState` lines
      1922–1965, `ThrottledInteraction` lines 2046–2065). These are
      changelogs in a reference doc.
- [ ] `CONCEPTS.md §7 Platform & parity` (lines 2606–2680): delete; itself
      points at `CLAUDE.md` as authoritative, then duplicates the parity
      surface anyway.
- [ ] `CONCEPTS.md` "How to read an entry" (lines 38–56): delete the
      meta-template explanation; readers figure it out from the first
      entry.
- [ ] `lib/baumhard/todo.txt`: delete the file. Fold the 1–2 still-relevant
      items into `TODO.md`.
- [ ] `format/migration.md` "TextRun ranges: code points → grapheme clusters"
      (lines 99–137): delete; migration story specific to a finished commit,
      readers never need it again.

### 1.2 Delete dead code
- [ ] `Renderer` dead fields: `instance`, `adapter`, `surface_capabilities`,
      `texture_format`, `shaders`, `render_pipeline`, plus the
      `default_pipeline` build path inside `Renderer::new`
      (`src/application/renderer/mod.rs:250-266` and `:565-645`). Verify
      with `grep -rn 'self\.\(instance\|adapter\|shaders\|render_pipeline\)\b'`
      — should be zero hits after.
- [ ] `lib/baumhard/src/shaders/shaders.rs:16-19`: collapse the two
      identical `SHADERS` table entries (both point at `test_shader.wgsl`)
      to one inline shader load.
- [ ] `renderer/tree_buffers.rs` keyed-rebuild branch: confirm it has no
      callers (per `scene_buffers.rs:74-77` it does not), rip the keyed
      branch, the `dirty_node_ids: Option` parameter, and the `seen`
      bookkeeping — keep only the slow (full) path.
- [ ] `color_picker_overlay/picker_glyph_areas/sections/title.rs` and
      `hint.rs`: delete files. Remove `title_template_*` / `hint_text_*`
      from `widgets/color_picker.json`. Remove `title_pos` / `hint_pos`
      from `color_picker/layout.rs:57-59` and the
      `compute_positions.rs:103-107` calls.
- [ ] `color_picker_overlay/glyph_model.rs`: delete file plus the
      `glyph_model_from_picker_area` model-attach loop in `trees.rs:53-59`.
      The renderer explicitly ignores these `GlyphModel` children — pure
      speculative seam (`CODE_CONVENTIONS §5 §7`).
- [ ] `renderer/borders.rs:99-123, 133-157`: delete
      `create_border_buffer_spans` and `create_centered_cell_buffer` —
      grep finds zero callers.
- [ ] `renderer/hit.rs:222-292`: delete 3 of the 4 trivial
      `find_first_aabb_hit_*` tests; keep the one substantive case.
- [ ] `renderer/tree_walker.rs:200-258`: delete the two
      `shape_one_element_*_yields_*` iterator-emit-count tautologies.
- [ ] `crates/maptool/src/main.rs:269-271`: inline `show_node` at the
      single call site (`run` line 129); delete the helper. The
      corresponding `show_returns_text_for_known_id` test moves to
      baumhard.
- [ ] `crates/maptool/src/convert/sections.rs:64-66`: delete
      `migrate_one_node_legacy` (alias of `migrate_one_node`); promote
      `migrate_one_node` to `pub(super)`.
- [ ] `lib/baumhard/src/gfx_structs/camera.rs:216-307`: delete the inline
      `#[cfg(test)] mod tests` (duplicate of `tests/camera_tests.rs`).
      Migrate the two `apply_mutation_*` tests over first if they're
      unique to the inline block.
- [ ] `mindmap/scene_builder/builder.rs:105-137`: delete the 8-line
      `build_scene` and `build_scene_with_offsets` wrappers; callers use
      `build_scene_with_cache` directly with
      `SceneSelectionContext::default()`.
- [ ] `gfx_structs/tree_walker.rs:74-76`: delete `walk_tree`; callers
      use `walk_tree_from(g, m, g.root, m.root)` (or — better — promote
      it to a method on `Tree`).

### 1.3 Tests that test framework / type system (delete)
- [ ] `lib/baumhard/src/util/tests/primes_test.rs`: replace 1242 lines
      with `for n in 2..10000 { assert_eq!(is_prime(n), reference_table.contains(&n)); }`
      (~10 lines, also adds the negative side).
- [ ] `lib/baumhard/src/util/tests/ordered_vec2_tests.rs:5-11`:
      `equals` test asserts `PartialEq` derive; delete.
- [ ] `gfx_structs/tests/element_tests.rs:62-84, 113-129, 134-146,
      167-182, 184-198`: delete or merge — all are getter / `PartialEq` /
      `Default` / `Clone` derive tests.
- [ ] `gfx_structs/tests/spatial_descend_tests.rs:307-340`: merge the
      `MouseEventData::new`/`zero`/`extreme_values` trio into one assertion.
- [ ] `gfx_structs/tests/zoom_visibility_tests.rs`: drop redundant
      `min_only_is_inclusive` / `max_only_is_inclusive` /
      `closed_window_renders_inside_band` /
      `single_point_band_is_inclusive` quadruplet to one parametric test;
      delete `test_default_is_unbounded`.
- [ ] `gfx_structs/tests/area_tests.rs:40-48` `test_outline_default_is_none`:
      delete (covered implicitly by non-default cases).
- [ ] `mindmap/animation.rs:259-273`
      `test_animation_timing_serde_round_trip`: tests `serde` itself;
      delete or strengthen with non-`None` `then`.
- [ ] `console/tests/state.rs:9-32` and `console/tests/grapheme.rs:13-21`:
      delete the two getter-and-sentinel tests.
- [ ] `font/tests/metrics_tests.rs:21-24, 45-55`: delete two pin-the-constant
      tests.
- [ ] `mindmap/loader.rs:582-619`: consolidate `test_save_blank_map_round_trip`
      and `test_save_to_file_round_trip` to one parametric.
- [ ] `mindmap/model/tests.rs`: collapse the six near-identical
      `*_zoom_window_round_trips` tests (lines 532, 549, 566, 584, 608,
      632) to one parametric test over the four `*ZoomWindow*`-bearing
      structs.
- [ ] `crates/maptool/src/main.rs:629-697`: drop the `grep_regex_*` tests
      that exercise the `regex` crate (keep one smoke).
- [ ] `crates/maptool/src/verify/mod.rs:80-117`: keep only the `Display`
      test, drop the three constructor-getter tests.
- [ ] `keybinds/tests.rs`: collapse the six
      `test_default_config_has_*_actions` tests into one (or two: document vs
      modal) table-driven test.
- [ ] `console/commands/mod.rs:101-144`: drop registry-presence tests
      (`cargo build` already enforces the const slice).
- [ ] `macros/tests.rs:285-325`: collapse the four JSON-shape pinning tests
      to one round-trip plus the existing default-target test.
- [ ] `app/tests.rs:113-161`: delete `test_double_click_guard_*` —
      reproduces the production predicate inline (passes regardless of
      drift).

### 1.4 Verification for Batch 1
- `./test.sh` passes with the same number of file-load tests, fewer assertion
  tests; check `target/llvm-cov` if running coverage to confirm only dead
  branches lost coverage.
- Manually open `maps/testament.mindmap.json` on native and WASM; open the
  color picker (no title/hint regression visible because they were never
  drawn).
- Diff `wc -l` before/after to confirm ~3000 LoC removed.

---

## Batch 2 — One canonical Color (correctness fix) — SHIPPED

Cross-batch dependency: lands before Batches 4 and 6 to avoid touching the
same files twice.

The audit found three `Color`-shaped types in two crates plus open-coded
`u8↔f32` quantisation helpers (six in total — five flagged in the audit
plus a sixth in `tree_builder/node.rs` surfaced during review), one of
which (`Color::to_float`) was arithmetically wrong. Net goal: one
canonical 8-bit-per-channel `Color` in `baumhard::util::color`, every
quantisation routed through `convert_f32_to_u8` /
`convert_u8_to_f32` (mirror primitives in
`baumhard::util::color_conversion`), and the cosmic-text bridge
contained inside `baumhard::font::hex` as free functions.

**API-shape choice (departure from initial audit).** The audit
initially proposed `Color::from_rgb_f32` / `to_rgba_f32` /
`from_cosmic` / `Into<cosmic_text::Color>`. Instead the implementation
chose free functions in `baumhard::font::hex`
(`cosmic_color_from_rgba` / `cosmic_color_to_rgba`). Reasoning:
adding `Color::from_cosmic` would force `util::color::Color` to know
about `cosmic_text::Color`, breaking the dependency boundary that
`util/` carefully maintains (cosmic-text only enters `font/`). The
free-fn-in-`font::hex` shape matches the existing
`hex_to_cosmic_color` pattern.

- [x] Fix `Color::to_float` — now routes through `convert_u8_to_f32`.
- [x] Audit every `Color::to_float` call site — one production caller
      (`matrix.rs:199`) confirmed dead today (only reachable through
      `GlyphModel`, which `tree_walker.rs` skips); three test callers
      used `Color::black()` so the bug was never visible. Documented
      in commit message.
- [x] Replace `color_picker_overlay/color.rs::rgb_to_cosmic_color`
      body with `cosmic_color_from_rgba`. The 3-line wrapper itself
      was KEPT — 9 call sites in the picker need an opaque RGB triple,
      and pinning `alpha = 1.0` once via this domain-named wrapper is
      cleaner than `cosmic_color_from_rgba([r, g, b, 1.0])` everywhere.
- [x] Replace `dynamic_context::cosmic_to_rgba` with
      `cosmic_color_to_rgba` directly at the call site (function
      deleted).
- [x] `glyph_model.rs:65-75` byte-quantisation loop — moot (file
      deleted in Batch 1).
- [x] Replace inline `[r,g,b,a]/255.0` conversions at
      `console_pass.rs:112-117`, `console_pass.rs:359-365`
      (`to_rgba` closure inlined twice), `render.rs:125-130` (now
      `convert_u8_to_f32(&rect.color)`).
- [x] **Picker `make_area.rs:52-57` cosmic→f32 inline** — surfaced by
      review, replaced with `cosmic_color_to_rgba(style.color)`.
- [x] **`tree_builder/node.rs:223-228` f32→u8 open-coded loop** —
      surfaced by review, replaced with `BaumhardColor::new_f32(&fc)`
      (which routes through `convert_f32_to_u8`).
- [ ] Duplicate-`Color` policy decision (deferred to Batch 3.1 by
      design — the plan's own recommendation said "item list in
      Batch 3"). The two `Color`s coexist today via the new
      `font::hex` bridge functions, the same pattern
      `hex_to_cosmic_color` uses. Removing the `font::Color`
      re-export is part of growing the `font/` wrapper in Batch 3.1.

---

## Batch 3 — Wrapper consolidation (mechanical, high leverage)

Cross-batch dependency: Batches 4–8 read more naturally on top of this. Each
sub-batch (3.1–3.5) is independently shippable.

### 3.1 `cosmic_text` — grow the `font/` wrapper
The wrapper exists (`lib/baumhard/src/font/`) but the renderer constructs
`cosmic_text::Buffer/Metrics/Shaping/Align/Color` directly at ~12 sites.

- [ ] Add `font::buffer::create(metrics, attrs, shaping) -> Buffer` factory
      that bundles the typical `Metrics::new(font_size, line_height) +
      Buffer::new + set_attrs/set_text_with_shaping` shape.
- [ ] Add `font::Color::WHITE`, `font::Color::BLACK` constants for the
      `cosmic_text::Color::rgba(255,255,255,255)` literals at
      `render.rs:99`, `mod.rs:817`, `tree_walker.rs:179`.
- [ ] Add `font::shaping::ADVANCED` and `font::align::CENTER` re-exports.
- [ ] Replace direct `cosmic_text::*` imports in
      `renderer/borders.rs`, `renderer/scene_buffers.rs`,
      `renderer/console_pass.rs`, `renderer/tree_walker.rs`,
      `renderer/render.rs`, `renderer/mod.rs:74,817` with the new factories.

### 3.2 `winit` — create the wrapper that doesn't exist
This is the largest leak in the workspace. ~15 sites import `winit::keyboard::*`,
`winit::event::*`, `winit::dpi::*`, `winit::window::*`.

- [ ] Create `src/application/platform/` module containing
      neutral `Key`, `Modifiers`, `MouseButton`, `KeyEvent`, `WindowSurface`
      types. The neutral types should compile on both native and WASM
      without `winit` being a leaf dep.
- [ ] `platform::translate::from_winit_key(...)` is the only place
      `winit::keyboard::Key` is touched.
- [ ] Replace `winit` imports across `keybinds/bind.rs:15,196,197`,
      `app/text_edit/editor.rs:14`, `app/label_edit.rs:8,515,681,700`,
      `app/event_keyboard.rs:10,11`, `app/event_mouse_click.rs:10`,
      `app/event_cursor_moved.rs:12,13`, `app/input_context.rs:13`,
      `app/input_context_core.rs:34`, `console_input/dispatch.rs:9`,
      `color_picker_flow/click.rs:7`, `app/mod.rs:126,393`. Renderer takes
      `(width: u32, height: u32) + impl SurfaceProvider` rather than
      `winit::dpi::PhysicalSize` + `winit::window::Window`.

### 3.3 `log` — single-source the logging facade
22+ files import `log` directly. `main.rs` initialises both `env_logger` and
`console_log` directly.

- [ ] Add `lib/baumhard/src/util/log.rs` re-exporting
      `info!/warn!/error!/debug!/trace!` plus a single
      `init_logger(target: LogTarget)` function that selects between
      `env_logger`/`console_log` based on platform.
- [ ] Replace all 22+ direct `use log::*;` imports with
      `use baumhard::util::log::*;` (or `use crate::log::*;` inside baumhard).
- [ ] Remove `env_logger::init()` (`main.rs:79`) and
      `console_log::init_with_level(...)` (`main.rs:89`) — call
      `baumhard::util::log::init()` once.

### 3.4 `wgpu` — bootstrap factories for the renderer
- [ ] Add `Renderer::bootstrap_native(window: &impl SurfaceProvider) ->
      Result<Self, RendererError>` and `Renderer::bootstrap_wasm(canvas)`
      factories that own the `wgpu::Instance` construction.
- [ ] Delete `use wgpu::Instance` from `app/run_native_init.rs:12` and
      `app/run_wasm/mod.rs:39,521`.

### 3.5 `serde_json` — thin format facade
- [ ] Add `lib/baumhard/src/format/json.rs` exposing
      `load_json<T>(s: &str) -> Result<T, JsonError>`,
      `save_json<T>(...)`, `Value` re-export.
- [ ] Convert `keybinds/config.rs:414`,
      `macros/loader/mod.rs:58,71,85,88,149,227`,
      `widgets/color_picker_widget.rs:161` to call the facade.
- [ ] `mindmap/loader.rs` is borderline (it IS the on-disk format wrapper);
      either bless it as the canonical loader or route it through
      `format::json`.

### 3.6 Atomic save (single-source from maptool to baumhard)
- [ ] Move `crates/maptool/src/main.rs:577-608::save_map +
      write_atomic` into `lib/baumhard/src/mindmap/loader.rs::save_to_file`.
      The atomic temp+rename and the `serde_json::Value`-routed pretty
      serializer become the canonical save (every editor session benefits).
- [ ] Delete the duplicate `write_atomic` at
      `crates/maptool/src/convert/portals.rs:125-142` (use the new helper).
- [ ] Update the test in `crates/maptool/src/main.rs:1313` to call the
      canonical helper; the byte-identical-across-runs test moves to
      baumhard.

---

## Batch 4 — De-duplicate logic

### 4.1 Spatial / geometry primitives
- [ ] Extract `node_center` to one place
      (`MindNode::center_vec2` exists at
      `lib/baumhard/src/mindmap/model/node.rs:214`); delete the two free
      functions at `mindmap/connection/mod.rs:95` and
      `mindmap/scene_builder/portal.rs:297`.
- [ ] Merge the BVH descent — `gfx_structs/tree.rs:329-398::bvh_descend` and
      `gfx_structs/tree_walker.rs:566-613::spatial_descend_recurse` are the
      same algorithm with one missing shape-refinement (the walker copy
      routes onto an ellipse via its AABB). Extract one
      `bvh_find(arena, root, point, slack, refine_with_shape)` helper.
- [ ] Merge `cubic_bezier_length` and `sample_cubic_bezier` (each walks
      the subdivision, in a tight loop) — extract
      `build_arc_length_table(start, c1, c2, end) -> Vec<f32>`
      (`mindmap/connection/bezier.rs:53-99`).
- [ ] Single-source `apply_drag_delta` and
      `apply_drag_delta_and_collect_patches` (4 functions → 2;
      `document/hit_test.rs:386-466, 706-759`); the patch-collecting
      variant is the superset.

### 4.2 Connection / label scene-building
- [ ] Extract `compute_label_layout` from the duplicated
      synthesised-label pass (`mindmap/scene_builder/label.rs:177-253`,
      ~80 lines repeated verbatim).
- [ ] Extract `emit_connection_element` from the cache-hit / translate /
      slow paths (`mindmap/scene_builder/connection.rs:155-192, 259-308,
      376-408`) — same `cap_start/cap_end + glyph_positions` filter +
      `ConnectionElement` push three times.

### 4.3 Color picker section builders
- [ ] Parameterise `build_crosshair_arm_section` covering both
      `sat_bar.rs` and `val_bar.rs` (90% identical bodies).
- [ ] Move "value→cell-index" math to `color_picker/glyph_tables.rs`
      next to its inverse and call from
      `dynamic_context.rs:217-222`, `sections/sat_bar.rs:34-36`,
      `sections/val_bar.rs:33-35`.
- [ ] Extract `apply_ink(base, before_arm, after_arm, i, fs)` for the
      per-cell ink-offset compute (`compute_positions.rs:60-81`,
      duplicated for sat and val).

### 4.4 Renderer / scene_buffers
- [ ] Replace `tree_buffers.rs:156-192::rebuild_node_backgrounds_from_tree`
      with a `yield_background` closure passed to `tree_walker.rs:85-117`
      (the walker already supports the shape).

### 4.5 WASM / native event-handler convergence (Track A.3)
The single largest duplication in the codebase. `WASM_CONVERGENCE.md` calls
this out as remaining work; cleanup goes in two passes.

- [ ] Extract `cross_dispatch::handle_double_click(ctx, hit) -> ...` from
      `dispatch/native.rs:364-479` (Node/Portal/EdgeLabel arms);
      `run_wasm/event_mouse_click.rs:98-235` calls the helper instead of
      duplicating the body.
- [ ] Extract `app/click.rs::handle_click_outside_commit` and
      `handle_release_selection` from the
      `event_mouse_click.rs:534-554` and
      `run_wasm/event_mouse_click.rs:283-425` duplicate sites.
- [ ] Extract `now_ms()` to `application/common/` (`app/mod.rs:131-144`
      defines it twice with platform `cfg`).

### 4.6 Console / dispatch
- [ ] Promote `console/parser.rs::is_kv_token` to `pub(super)`; delete the
      duplicate at `console/completion.rs:218-232`.
- [ ] Move `console/commands/zoom.rs:202::finalize` and
      `font.rs:467::finalize` to one
      `console/helpers.rs::applied_or_no_change(verb, kind, changed)`.
- [ ] Replace `console/commands/font.rs:367-465::section_font_outcome` /
      `node_font_outcome` with the existing `helpers::ApplyTally`.
- [ ] Fold `dispatch/traits/dispatch.rs:131-192::apply_kvs` and
      `apply_to_targets` aggregation tails into one
      `OutcomeTally::into_report(label_for_messages: Option<&str>)`.

### 4.7 Cross-document doc deduplication
- [ ] Privilege-gate paragraphs: pick `format/macros.md` as canonical;
      `CODE_CONVENTIONS §3` and `CONCEPTS.md "Action dispatch" §5` link
      instead of restating.
- [ ] Mutation-first rule: keep `lib/baumhard/CONVENTIONS.md §B2` only;
      `CONCEPTS.md §1` and `CODE_CONVENTIONS.md §1` link.
- [ ] Cross-platform parity: keep `CODE_CONVENTIONS.md §4` only.
- [ ] Custom mutations / `var(--name)` collapse: keep `format/sections.md`
      only; trim the `CONCEPTS.md` `MindSection` block.
- [ ] Zoom bounds cascade: keep `format/zoom-bounds.md`; trim
      `CONCEPTS.md` to one sentence.

---

## Batch 5 — God-functions and god-structs

### 5.1 Renderer
- [ ] Split `Renderer::new` (`renderer/mod.rs:529-706`) into
      `init_text_pipeline`, `init_rect_pipeline`, `default_buffer_state`.
      Make `pipeline::create_rect_pipeline(device, format)` actually take
      the rect-pipeline construction (currently inlined in `new`).
- [ ] Split `Renderer::render` (`renderer/render.rs:84-378`) into
      `bake_main_rects`, `bake_overlay_rects`,
      `upload_rect_vertices() -> (u32, u32)`, `record_pass`.
- [ ] Replace `unsafe { from_raw_parts }` for vertex upload with
      `bytemuck::cast_slice` (`render.rs:194-209`); 60×/sec hot path.
- [ ] Pre-size or reuse the `Vec<TextArea>` in `main_text_areas`
      (`render.rs:218-243`); currently a fresh `Vec` from six chained
      iterators per frame.
- [ ] Cache `(width, height)` rounded to char cells for
      `rebuild_selection_rect_overlay` (`scene_buffers.rs:230-289`); 4
      buffer shapings per drag tick today.

### 5.2 Cursor / event handling
- [ ] Split `handle_cursor_moved` (`event_cursor_moved.rs:27-545`) into:
      picker/hover gate, a `promote_pending_drag(ctx, hits) -> Option<DragState>`
      helper that absorbs the 6-way `Pending`-promotion ladder (lines
      196-414), and per-drag-arm methods.
- [ ] Convert `DragState::Pending`'s 8 `Option` fields
      (`app/mod.rs:459-510`) to
      `enum PendingHit { Node, EdgeLabel(...), PortalLabel(...),
      EdgeHandle(...), NodeResize(...), SectionResize(...) } + start_pos`.
- [ ] Lift the type-to-edit branch out of `event_keyboard.rs:204-304`
      into `try_type_to_edit(ctx, key, key_name) -> bool`.

### 5.3 Document / state
- [ ] Pull `route_label_edit_key` out of `app/mod.rs:387-413` into
      `label_edit.rs` (the rest of label edit lives there).
- [ ] Split `MindMapDocument`'s 12 public fields into
      sub-bundles (`mutations: MutationState`, `previews: PreviewState`,
      `animations: AnimationState`); narrow accessors. Delete
      `from_finalized_mindmap` test bypass and make `grow_*` cheap enough
      that production and tests share a single constructor path.
- [ ] Carve `ModalState { console, label_edit, portal_text_edit,
      color_picker }` from `InputHandlerContext`'s 21 fields
      (`app/input_context.rs:37-93`); narrow dispatch borrows.
- [ ] Convert `ConsoleEffects`'s 8 mutually-exclusive `Option` fields
      (`console/mod.rs:56-114`) to one
      `enum ConsoleSideEffect { OpenLabelEdit(EdgeRef),
      OpenColorPicker(...), CloseColorPicker, ... }` field.

### 5.4 Predicate / dispatch
- [ ] Refactor `Predicate::test` (`gfx_structs/predicate.rs:176-485`):
      extract `evaluate_field(elt, field, comp) -> bool`; the outer body
      becomes a fold over `(field, comparator)` pairs.
- [ ] Replace `KeybindConfig::resolve` (`keybinds/config.rs:420-795`,
      375 lines) with a `#[derive(KeybindConfig)]` macro on `Action` that
      walks `ActionKind` to emit fields, default, and resolve table.
      Mandala already has `lib/mandala_derive/`.
- [ ] Replace `console/commands/font.rs::execute_font` (185 lines,
      `:174-358`) with `parse_font_args -> FontArgs` + `apply_font_args
      -> ApplyTally` + one finalise site, mirroring how `font set` and
      every other `console/traits/` axis already dispatches.
- [ ] Table-drive the six `Has*Color`/`Accepts*`/`Handles*` impls in
      `console/traits/view.rs` (926 lines, six 7-arm matches with shared
      `is_valid_color_literal` helper).

---

## Batch 6 — Types that don't lie

### 6.1 Panic on interactive paths (CODE_CONVENTIONS §9)
- [ ] `AnimationInstance::timing()` (`document/types.rs:69-74`) —
      `.expect(...)` on a per-frame interactive path. Store
      `AnimationTiming` directly on `AnimationInstance`, carved from
      `cm.timing` at construction. The `Option<AnimationTiming>` only
      needs to live on `CustomMutation`, not on the live instance.

### 6.2 Lying types
- [ ] `Color::to_float` (already in Batch 2) — was returning `[0,0,0,1]`
      while pretending to convert.
- [ ] `GlyphMatrix::IndexMut` (`gfx_structs/model/matrix.rs:37-54`):
      auto-grow on read-style indexing. Move auto-grow to an explicit
      `ensure_line(i)` method; `IndexMut` panics like `Vec`.
- [ ] `InputContext::parent` (`keybinds/context.rs:43-45`): unconditionally
      returns `Document` regardless of `self`. Either rename to
      `document_root()` or inline at the single call site
      (`resolved.rs:123`).
- [ ] `ColorPickerPreview` enum (`document/mod.rs:163-169`) has one
      variant. Either collapse to a struct or commit to additional variants
      — don't promise polymorphism that doesn't exist.

### 6.3 Test-time bypasses
- [ ] Remove `MindMapDocument::from_finalized_mindmap`
      (`document/mod.rs:428-431`); tests and production share `from_mindmap`.
      The motivation (FONT_SYSTEM contention) is the bug to fix —
      either memoise `grow_*` or run finalize once per test fixture and
      clone.
- [ ] `tests_common::doc_with_one_orphan_node`
      (`document/mod.rs:124-144`) constructs `MindMapDocument` field-by-field;
      replace with a `MindMapDocument::with_orphan(id, pos)` constructor in
      `defaults.rs`.

### 6.4 Lock soup
- [ ] `RegionParams` (`gfx_structs/util/regions.rs:53-294`): six
      independent `RwLock<usize>` fields always written together, six
      `read_*` accessors that return the same `RegionError`. Collapse
      to one `RwLock<RegionParamsInner>`. Drop the `Updating` error
      variant (the doc-claimed "writers and readers without a global
      mutex" benefit doesn't survive when readers take 4 sequential
      locks).
- [ ] `acquire_font_system_write` busy-wait (`font/fonts.rs:299-356`):
      replace 5-second 1ms-sleep poll with immediate `try_write` panic
      on contention (it's always re-entrancy on a single-threaded app).

### 6.5 Walker correctness
- [ ] Decide and document the sibling-channel ordering invariant
      (`gfx_structs/tree_walker.rs:264-314 align_child_walks`). Either
      `debug_assert!` ascending channels at insert points, or sort
      children at apply time.
- [ ] Rewrite `walk_tree_from` recursion + `compare_apply_repeat_while`
      with `while let Some(...)` and an explicit non-recursive driver
      (`gfx_structs/tree_walker.rs:32-225`). Recursion depth is bounded
      by tree depth; current code uses pointer-style `unwrap()` chains.
- [ ] `apply_to_area(Event(_))` / `apply_to_model(Event(_))` silent-drop
      (`gfx_structs/mutator.rs:317-350`): make `Event` a newtype that
      doesn't compose into `apply_to_area/_model` paths at all.

### 6.6 Event-handling invariants
- [ ] `Instruction::RotateWhile` empty arm
      (`gfx_structs/tree_walker.rs:159`) — at minimum `log::warn!` on
      attempted use; better, remove the variant if nothing dispatches.
- [ ] Macro inline-id non-determinism (`macros/loader/mod.rs:138-181`):
      the doc-comment warns about HashMap iteration order then ships
      anyway. Either auto-prefix inline ids with the node id (deterministic),
      or reject the load — `CODE_CONVENTIONS §5` forbids "half-features".
- [ ] Console tokenize escape semantics (`console/parser.rs:25-67`):
      `\n`/`\t`/unknown escapes silently produce a literal `\`. Either
      reject unknown escape sequences or document the semantics.

### 6.7 Hot-path allocations
- [ ] Replace `EdgeKey`'s 3 `String` fields
      (`mindmap/scene_cache.rs:56-62`) with `Cow<'static, str>` for
      `edge_type` (only 3 values: `parent_child`, `cross_link`,
      `parent_child_no_inherit`) and an `EdgeRef<'a>` borrow type that
      hashes the same as `EdgeKey` for cache lookups.
- [ ] Re-key `mindmap_buffers` and friends from `unique_id.to_string()`
      to `usize` (`renderer/tree_buffers.rs:42-44, 90-92, 100-102`).
- [ ] `display_text` returns `Cow<'_, str>`; specialise the 1-section
      case to borrow (`mindmap/model/node.rs:192-201`).
- [ ] Cache the four legacy preset side-patterns as `static LazyLock`,
      or fold into a `SidePattern::SingleClusterChar(char)` variant
      (`mindmap/border.rs:597-605`).
- [ ] Move `NodeShape::from_style_string` parsing onto `MindNode` (or
      stamp on `BorderNodeData` upstream); currently re-parsed per node
      per frame (`mindmap/tree_builder/border.rs:96`).
- [ ] Rebuild `node_map`/`section_map`/`section_counts` reverse maps
      during `append_node_sections` instead of a post-pass clone of every
      key string (`mindmap/tree_builder/mod.rs:160-171`).
- [ ] Mutate `ColorFontRegions` ranges in place via `BTreeMap::range`
      instead of clone-then-extend (`core/primitives.rs:202-329`, four
      call sites).

### 6.8 Loader streaming
- [ ] `mindmap/loader.rs:38-98`: switch from
      `serde_json::Value` peek-then-`from_value` (two heap-resident
      copies) to direct `MindMap`/`MindNode` deserialisation with
      `#[serde(deny_unknown_fields)]` + an explicit legacy-detector
      that runs on `serde_json::Error` rather than on a happy-path
      pre-parse. Doc-comment claim of "no second parse" matches the new
      shape.

---

## Batch 7 — Test surface remediation (the inverse problem)

Add tests where complexity is high and coverage is thin. After Batch 1
removed ~35 weak tests, the suite is leaner; this batch adds the targeted
tests that pin actual invariants.

- [ ] **`mindmap/portal_geometry.rs`** (339 LoC, 0 inline tests): add
      `mindmap/tests/portal_geometry_tests.rs` covering anchor resolution,
      pair endpoint inversion, and offset application.
- [ ] **`mindmap/scene_cache.rs`** (439 LoC, 0 cache-primitive tests):
      add `EdgeKey::new`, `clear`, eviction-on-edge-deletion, hit/miss
      counter tests.
- [ ] **`document/mutations/flower_layout.rs` &
      `mutations/tree_cascade.rs`**: add per-variant undo round-trip
      tests (`TEST_CONVENTIONS §T7`).
- [ ] **Macro privilege fail-closed tests** (`format/macros.md`'s
      threat model): assert `dispatch_macro` rejects `App`/`Map`/`Node`
      tier sources on `ConsoleLine` and destructive `Action`s.
- [ ] **`mindmap/animation.rs::tick_animations` + easing curves**: add
      sample tests on the curve outputs and a `tick_animations` driver
      test (currently only the JSON wire format is covered).
- [ ] **`Color::to_float` round-trip** (after Batch 2): property test
      asserting `from_rgb_f32 ∘ to_rgba_f32` round-trips within
      `1.0/255.0`.
- [ ] **`event_subscribers`** (`gfx_structs/tests/element_tests.rs:202-238`):
      strengthen — drive an event through the element and assert the
      subscriber observed it (current test only asserts `Vec::push`).
- [ ] **`measure_glyph_ink_bounds_x_offset_from_advance_center`**
      (`font/tests/fonts_tests.rs:94-109`): assert Tibetan svasti's
      offset is non-zero and the Latin "A"'s offset is small (the
      documented motivating bug); current test only rejects NaN.

---

## Batch 8 — Documentation surface

After Batches 1, 3.1, and 4.7 the prose has shed ~75 KB of stale and
duplicated content. This batch adds what's missing.

- [ ] **Workspace `README.md`** (currently 0 bytes): write 30–60 lines —
      what Mandala is, build/run quickstart (`./test.sh`, `./build.sh`,
      `./run.sh`), links to `CLAUDE.md`, `CONCEPTS.md`, `CODE_CONVENTIONS.md`,
      `format/`. This is what GitHub renders to first-time visitors.
- [ ] **`lib/baumhard/readme.md`**: either expand to a real crate-level
      README (link to `CONVENTIONS.md`, key modules: `mindmap/`,
      `gfx_structs/`, `font/`) or delete and rely on rustdoc.
- [ ] **`CONCEPTS.md`** §1 stances (lines 74–161): reduce each stance to
      a one-liner + the cross-doc link rather than 4–8 sentences each
      (saves ~3 KB).
- [ ] **`CONCEPTS.md` boilerplate "Summary./What it's for./Under the hood./
      Vision./Caveat." labels** on every concept entry: drop the labels;
      rely on paragraph order. The convention stays; the typography goes
      (saves ~6 KB across ~120 entries).
- [ ] **Per-file `//!` headers** that restate the file list rather than
      the concept (`lib/baumhard/src/mindmap/mod.rs:11-40`,
      `lib/baumhard/src/lib.rs:15-34`, etc.): trim per `CONVENTIONS.md
      §B9`.
- [ ] **Inline meta-commentary** in `app/mod.rs:43-48, 92-109`,
      `app/event_mouse_click.rs:97-117`, `document/mod.rs:206-291`
      (lock-scope discipline note that's a half-screen long): trim to
      one-line invariant statements where appropriate, move to module
      `//!` where load-bearing.

---

## Critical files (one-stop reference for the implementer)

Files most heavily modified across batches — keep these open during work:

- `lib/baumhard/src/util/color.rs` (Batches 2, 6)
- `lib/baumhard/src/font/{mod.rs,attrs.rs,fonts.rs,hex.rs,metrics.rs}` (Batch 3.1)
- `lib/baumhard/src/mindmap/loader.rs` (Batches 3.6, 6.8)
- `lib/baumhard/src/mindmap/scene_builder/{label.rs,connection.rs,builder.rs}` (Batches 1, 4)
- `lib/baumhard/src/mindmap/scene_cache.rs` (Batch 6.7)
- `lib/baumhard/src/gfx_structs/{tree.rs,tree_walker.rs,mutator.rs,predicate.rs}` (Batches 4.1, 5.4, 6.5)
- `lib/baumhard/src/gfx_structs/util/regions.rs` (Batch 6.4)
- `lib/baumhard/src/util/log.rs` (NEW — Batch 3.3)
- `src/application/platform/` (NEW module — Batch 3.2)
- `src/application/renderer/{mod.rs,render.rs,scene_buffers.rs,tree_buffers.rs,borders.rs,console_pass.rs,tree_walker.rs,pipeline.rs}` (Batches 1, 2, 3.1, 3.4, 5.1)
- `src/application/app/{mod.rs,event_cursor_moved.rs,event_keyboard.rs,event_mouse_click.rs,input_context.rs,label_edit.rs,run_native_init.rs,run_wasm/mod.rs,run_wasm/event_mouse_click.rs}` (Batches 3.2, 3.4, 4.5, 5.2, 5.3)
- `src/application/keybinds/{config.rs,bind.rs,context.rs}` (Batches 3.2, 3.5, 5.4, 6.2)
- `src/application/console/{parser.rs,completion.rs,mod.rs,commands/font.rs,commands/zoom.rs,traits/view.rs,traits/dispatch.rs}` (Batches 4.6, 5.3, 5.4)
- `src/application/color_picker_overlay/{color.rs,glyph_model.rs,picker_glyph_areas/sections/*}` (Batches 1.2, 2, 4.3)
- `crates/maptool/src/{main.rs,convert/portals.rs,convert/sections.rs,verify/mod.rs}` (Batches 1.2, 1.3, 3.6)
- `lib/mandala_derive/src/` (Batch 5.4 — KeybindConfig derive)
- `TODO.md`, `WASM_CONVERGENCE.md`, `CONCEPTS.md`, `lib/baumhard/todo.txt`,
  `format/migration.md`, `README.md`, `lib/baumhard/readme.md` (Batch 1.1, 8)

## Existing primitives to reuse (reminder, do not re-implement)

- `MindNode::center_vec2` (`mindmap/model/node.rs:214`) — the canonical
  AABB-centre for a `MindNode`.
- `convert_f32_to_u8` (`util/color_conversion.rs`) — saturating f32→u8
  per channel; reuse for all quantisation.
- `Color::new_f32` (`util/color.rs:234`) — `[f32;4] → Color`.
- `helpers::ApplyTally` (`console/helpers.rs`) — outcome aggregation;
  the hand-rolled `*_outcome` helpers in `font.rs` should call this.
- `OrderedFloat` (already in deps via `ordered-float`) — for any
  `f32`-as-key needs (instead of open-coding `f32::to_bits` equality).
- `payload_within_cap` (`user_config/mod.rs`) — file-size guard for
  user-config loads (the desktop loader currently rolls its own).
- `crossbeam-channel` and `slab` are in deps; check before adding new
  channel/arena primitives.
- `lib/baumhard/src/font/hex.rs` — the blessed `cosmic_text::Color`
  bridge; halo color paths should not bypass it.
- `lib/mandala_derive/` — proc-macro crate already exists for the
  `KeybindConfig` derive in Batch 5.4.

## Verification recipe (per batch and end-to-end)

After every batch:

1. `./test.sh` — passes; expect total test count to drop by ~30–50 in
   Batch 1 (deletions of weak tests), then rise modestly in Batch 7.
2. `./test.sh --lint` — `cargo fmt --check` clean, `cargo clippy` clean
   (advisory; treat new warnings as findings).
3. `./test.sh` also type-checks `wasm32-unknown-unknown`; cross-platform
   drift fails the run. Critical for Batches 3.2, 3.4, 4.5.
4. `./test.sh --bench` — for Batches 4 and 6.7 in particular, compare
   criterion deltas on the affected scene-build / cache benches.
5. **Manual smoke**: `./run.sh maps/testament.mindmap.json`; in both
   native and WASM, exercise: open, drag, zoom (ctrl+scroll), click an
   edge label, double-click a node, open the console, open the color
   picker, undo a mutation. The renderer and dispatch layer cannot be
   covered by the test suite alone.
6. **`maptool verify` against every map in `maps/`**: structural sanity
   that no on-disk format change slipped in.
7. After Batch 1: `wc -l` before/after to confirm the LoC drop matches
   expectations (~3000 LoC of code + ~75 KB of prose).
8. After Batch 6: smoke-test a session that creates and commits to many
   animated transitions; the `AnimationInstance::timing` change is the
   most user-visible correctness fix.

## Out of scope (explicit non-goals)

- Adding new user-facing features.
- Migrating `.mindmap.json` schema.
- Reorganising `format/` per-document layout (only deletion of stale
  paragraphs).
- Touching `lib/baumhard/CONVENTIONS.md` substantively (it is the most
  load-bearing convention doc).
- Changing the GPU pipeline structure (only its construction shape and
  bootstrap factories).
- Replacing `wgpu`/`cosmic-text`/`winit` with alternatives — the
  wrappers exist precisely so this CAN be done later, but doing it now
  is a separate effort.

