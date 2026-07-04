# Cross-Cutting Review — Duplication / SSOT Audit (Mandala + Baumhard + maptool)

Scope: seams between modules and crates. Method: 12 systematic sweeps over src/, lib/baumhard/src/, crates/maptool/src/ with context reads on every judged hit. Deliberate non-findings honored: `pub mod tests;` without cfg(test) in baumhard, no custom error types, renderer's cosmic-text privilege, docs-vs-code British split.

## Sweep 1 — Constant duplication

### C1. Touch move-threshold claims to mirror the mouse drag threshold but has drifted (4px vs 5px)
Severity: P2 | Category: ssot | Confidence: high
Files: src/application/app/touch_gesture.rs:79-84; src/application/app/mod.rs:136-147; src/application/app/event_cursor_moved.rs:202,598
Evidence: touch_gesture.rs:79-84 — "Mirrors the existing mouse drag threshold in `event_cursor_moved.rs` (see `DRAG_THRESHOLD_SQ` there) — same value, same intent." then `pub const MOVE_THRESHOLD_PX: f64 = 4.0;`. mod.rs:147 — `const DRAG_THRESHOLD_SQ_PX: f64 = 25.0;` (= 5.0 px linear, and it lives in mod.rs, not event_cursor_moved.rs; the name in the comment is also wrong).
Why it matters: The doc comment asserts an invariant ("same value, same intent") that is false: 4.0 ≠ sqrt(25.0). §4 makes touch a peer of mouse; the click-vs-drag discrimination knob should be one constant. The stale pointer (wrong name, wrong file) shows the two sites are already drifting.
Fix: One canonical constant (e.g. `POINTER_DRAG_THRESHOLD_PX: f64 = 5.0` in app/mod.rs or platform/input.rs); derive `DRAG_THRESHOLD_SQ_PX` as `POINTER_DRAG_THRESHOLD_PX * POINTER_DRAG_THRESHOLD_PX` and use the linear constant in touch_gesture. If 4px-for-touch is intentional, fix the comment to say so and why.
Effort: S

### C2. Selection-highlight cyan defined four times, three near-equal values, two crates
Severity: P2 | Category: ssot | Confidence: high
Files: lib/baumhard/src/mindmap/mod.rs:51; src/application/document/types.rs:15; src/application/renderer/mod.rs:911-916; src/application/renderer/scene_buffers.rs:101-102
Evidence:
- baumhard: `pub(crate) const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";` (= 0,229,255) with the admission "The app crate's `document::types::HIGHLIGHT_COLOR` is the approximately-matching float-RGBA form".
- app: `pub const HIGHLIGHT_COLOR: [f32; 4] = [0.0, 0.9, 1.0, 1.0];` (0.9*255 = 229.5).
- renderer mod.rs:916: `Attrs::new().color(baumhard::font::Color::rgba(0, 230, 255, 255))` — hand-converted third copy (230, not 229), comment says "Same cyan as HIGHLIGHT_COLOR".
- renderer scene_buffers.rs:102: `hex_to_cosmic_color(&handle.color).unwrap_or(Color::rgba(0, 229, 255, 255))` — fourth copy (229).
Why it matters: CODE_CONVENTIONS §1: "Geometry, **color**, regions are Baumhard's ... Do not redefine any of these in the app crate." The four sites already disagree at the byte level (229 vs 229.5 vs 230); the "canonical active-affordance color" is exactly the kind of knowledge that must be single-sourced. `mindmap/mod.rs`'s constant is `pub(crate)`, which *forces* the app crate to duplicate.
Fix: Promote one canonical constant in baumhard (e.g. `pub const SELECTION_HIGHLIGHT: Rgba = [0, 229, 255, 255]` in `util::color` or `mindmap`), and derive: hex form via `rgba_to_hex`/const string, float form via `convert_u8_to_f32`, cosmic form via `cosmic_color_from_rgba`. Replace all four sites (types.rs re-exports the float form).
Effort: M

### C3. Default node font size (14pt) and line-height factor (1.2) duplicated across crates; app re-implements baumhard's effective-scale block
Severity: P2 | Category: duplication | Confidence: high
Files: lib/baumhard/src/mindmap/tree_builder/node.rs:73,141-147; src/application/document/mod.rs:268-276; src/application/document/custom/sync.rs:32-34
Evidence: node.rs:146-147 `let scale = if scale_max > 0.0 { scale_max } else { 14.0 }; let line_height = scale * 1.2;` — document/mod.rs:273-274 repeats the identical block (`else { 14.0 }`, `scale * 1.2`) for node auto-size measurement; node.rs's own comment says "Mirrors the same `max` posture in `grow_one_node_to_fit_text`" — sync by comment, not by call. sync.rs:34 adds `DEFAULT_TEXT_RUN_SIZE_PT: u32 = 14` ("Mirrors cosmic_text's 14pt fallback used at scene-build time"). node.rs:73 also hardcodes `GlyphArea::new(14.0, 14.0 * 1.2, ...)`.
Why it matters: §5 "never copy — one function called in two or more places." If baumhard's scene-build sizing ever changes (e.g. 1.2 → 1.25), the app's `grow_one_node_to_fit_text` measurement drifts from the actual render and auto-sized nodes clip text. The 14pt default also silently couples the text-run reverse-converter (sync.rs) to the tree builder.
Fix: In baumhard, add `pub const DEFAULT_TEXT_SIZE_PT: f32 = 14.0;`, `pub const LINE_HEIGHT_FACTOR: f32 = 1.2;` and a shared `pub fn effective_section_scale(section: &MindSection) -> f32` next to the tree builder; call it from tree_builder/node.rs, document/mod.rs, and express `DEFAULT_TEXT_RUN_SIZE_PT` in terms of the constant.
Effort: M

### C4. Camera fit padding fraction 0.05 repeated as a magic literal
Severity: P3 | Category: duplication | Confidence: high
Files: src/application/renderer/hit.rs:53, src/application/renderer/hit.rs:193
Evidence: two `padding_fraction: 0.05` literals in `fit_camera_to_scene` and `fit_camera_to_tree` (same file); the two functions also duplicate the min/max AABB fold over different element sources.
Why it matters: A future "fit to selection" (anticipated in the comment at hit.rs:198-201) will copy the literal a third time.
Fix: `const FIT_PADDING_FRACTION: f32 = 0.05;` in renderer/hit.rs (or a default on `CameraMutation::FitToBounds` in baumhard).
Effort: S

Clean in sweep 1: zoom limits (single `Camera2D::MIN_ZOOM/MAX_ZOOM`, all sites reference constants); throttle budget (`DEFAULT_BUDGET` = 14_000µs single-sourced; raw 14_000 appears only in its own tests); `DOUBLE_CLICK_MS`/`DOUBLE_CLICK_DIST_SQ` single-sourced and shared by native + wasm via `is_double_click`; `LONG_PRESS_MS` single; `EDGE_HIT_TOLERANCE_PX`/`HANDLE_HIT_TOLERANCE_PX` are two deliberately separate named knobs (same value today, independent by design); border geometry fractions (`BORDER_CORNER_OVERLAP_FRAC`, `BORDER_APPROX_CHAR_WIDTH_FRAC`) defined once in border.rs:26,34 and imported everywhere; color-picker cell counts single-sourced in color_picker/glyph_tables.rs and imported by the overlay.

## Sweep 2 — Color pipeline

Map of the pipeline: `lib/baumhard/src/util/color_conversion.rs` is the declared SSOT (`convert_f32_to_u8`, `convert_u8_to_f32`, `resolve_var`, `hex_to_rgba`, `hex_to_rgba_safe`, `hsv_to_rgb`, `rgb_to_hsv`, `hsv_to_hex`, `rgba_to_hex`, `hex_with_alpha_scaled`, `hex_to_hsv_safe`, `from_hex`, `add_rgba`); `font/color.rs` (`cosmic_color_from_rgba`/`to_rgba`) and `font/hex.rs` (`hex_to_cosmic_color`) delegate to it; the app's color picker (`color_picker/`, `color_picker_overlay/`, `color_picker_flow/`) and renderer consume only these. maptool does no color parsing. This is genuinely well-consolidated. Two exceptions:

### C5. `hex!` macro is a second, semantically different hex parser with zero production callers
Severity: P3 | Category: duplication, dead-code | Confidence: high
Files: lib/baumhard/src/util/color.rs:52-72 (definition); only callers: lib/baumhard/src/util/tests/color_tests.rs:15-19,29-32,85-86
Evidence: the macro re-implements parsing with `u8::from_str_radix(&color[i..i+2], 16).unwrap_or(0)` — per-channel silent-zero fallback, even-length only (no `#abc`/`#abcd` short forms), byte-slicing `color[i..i+2]` that panics on non-ASCII input (multi-byte UTF-8 at an odd boundary) — while color_conversion.rs's module doc declares itself "the single source of truth for how do we turn a hex string ... into a float quad", accepts 3/4/6/8, and rejects cleanly.
Why it matters: two parsers with different failure semantics inside the SSOT module pair; the macro's doc admits it is not const-capable ("Runs at evaluation time, not const time"), which removes its only reason to exist over `hex_to_rgba_safe`.
Fix: delete `hex!` and port its tests to `hex_to_rgba_safe`, or make it expand to `$crate::util::color_conversion::hex_to_rgba_safe($color, [0.0; 4])`.
Effort: S

### C6. Two hand-rolled color-literal validators in console/traits disagree with each other and with the canonical parser
Severity: P2 | Category: ssot | Confidence: high
Files: src/application/console/traits/color_value.rs:37-43; src/application/console/traits/view.rs:598-606; (canonical: lib/baumhard/src/util/color_conversion.rs:67-104); related var-ref recognizers: src/application/document/custom/sync.rs:72,307; lib/baumhard/src/util/color_conversion.rs:50-60
Evidence: `ColorValue::parse` accepts hex lengths `3|4|6|8`; `is_valid_color_literal` (paste path) accepts only `6|8` — so `color bg=#abc` succeeds via kv but pasting `#abc` onto the same field is rejected as Invalid. Both re-implement the shape check instead of `hex_to_rgba(s).is_some()`. `is_valid_color_literal` additionally recognizes `var(--name)` with its own grammar (`strip_prefix("var(--")`), while baumhard's `resolve_var` accepts `var(<anything>)` with trim, and sync.rs uses a third recognizer `starts_with("var(")` (twice).
Why it matters: §1 — color parsing is Baumhard's; §2 — parallel paths. The 3/4-digit acceptance split is a live user-facing inconsistency between two verbs of the same console.
Fix: add `pub fn is_valid_hex_color(s: &str) -> bool` (delegating to `hex_to_rgba`) and `pub fn is_var_ref(s: &str) -> bool` / `pub fn parse_var_name(s: &str) -> Option<&str>` to `baumhard::util::color_conversion`; use them in ColorValue::parse, is_valid_color_literal, and sync.rs; pick one accepted-length policy (the canonical parser's 3/4/6/8).
Effort: M

Clean in sweep 2: f32↔u8 quantization single-sourced (every `*255.0` / `/255.0` outside color_conversion.rs is in tests or the two macros); HSV math exists once; cosmic Color bridging exists once; `resolve_var` is the only var-resolution implementation (recognition, not resolution, is what C6 covers).

## Sweep 3 — Geometry / AABB

### G1. Point-in-AABB (closed interval) implemented three times; baumhard has no public primitive for it
Severity: P2 | Category: duplication, api-design | Confidence: high
Files: src/application/renderer/hit.rs:17-19 (`fn aabb_contains`); src/application/document/hit_test.rs:178-179 and :187 (two inline copies inside `point_in_node_aabb`); lib/baumhard/src/gfx_structs/scene.rs:93 (private `SceneEntry::contains`); canonical shape-aware arm: lib/baumhard/src/gfx_structs/shape.rs:117 (`NodeShape::contains_local`)
Evidence: renderer/hit.rs's own test comment concedes the pattern: "The four hit-test bodies all previously open-coded the `>=` / `<=` predicate; locking the boundary here prevents a future open-vs-closed drift" — i.e. the renderer consolidated locally, but document/hit_test.rs and baumhard's scene.rs still carry their own copies of the same closed-interval predicate.
Why it matters: §1 "Geometry ... [is] Baumhard's ... Do not redefine any of these in the app crate"; the open-vs-closed boundary decision is exactly the kind of knowledge that drifts (the codebase also contains a *deliberately different* strict/epsilon-inset variant, scene_builder/connection.rs:422-434, which is fine because it's documented — but that only works if the closed default is single-sourced).
Fix: add `pub fn aabb_contains(pos: Vec2, min: Vec2, max: Vec2) -> bool` (and optionally a pos+size flavor) to `baumhard::util::geometry` with the closed-interval doc + `do_*()` test + bench per §B3/§B7 discipline; replace the three sites (scene.rs delegates too).
Effort: S

### G2. Node-center computed by hand at four production sites despite `aabb_center` / `MindNode::center_vec2` existing for exactly this
Severity: P3 | Category: duplication | Confidence: high
Files: lib/baumhard/src/mindmap/scene_builder/edge_handle.rs:85-86; lib/baumhard/src/mindmap/portal_geometry.rs:119; src/application/app/edge_drag.rs:49-50; src/application/document/edges/structural.rs:277
Evidence: all four spell `Vec2::new(pos.x + size.x * 0.5, pos.y + size.y * 0.5)`; `util::geometry::aabb_center`'s doc explicitly designates itself for "anchor resolution paths, scene-builder portal-pair midpoint compute" and says "where a `MindNode` is in scope, prefer the method". edge_drag.rs and structural.rs even have the `MindNode` in scope (`from_node.pos_vec2()` … could be `from_node.center_vec2()`).
Why it matters: §5; also the "control points are offsets from node centers" convention is load-bearing (edge model semantics) — every hand-rolled center is a chance to anchor an offset to the wrong point.
Fix: mechanical replacement with `aabb_center` / `center_vec2`.
Effort: S

### G3 (inventory). Five AABB representations; conversions cluster at the scene-builder and renderer walls
Severity: P3 | Category: api-design | Confidence: med
Files: `(Vec2, Vec2)` min/max tuples (renderer hitbox maps hit.rs:22,89,101,114; `Tree::aabb_cache` tree.rs:147; `subtree_aabb`; scene_builder/connection.rs:422); pos+size `OrderedVec2` pairs (`GlyphArea.position`/`render_bounds`, area.rs); `BoundingRectangle { delta_x, delta_y, length, width }` (offset+size, gfx_structs/util/hitbox.rs:54); `(f32,f32)` position/size pairs (`RenderScene` elements); f64 `position{x,y}` + `size{width,height}` (`MindNode`, serde shape).
Evidence: fit_camera_to_scene / fit_camera_to_tree (renderer/hit.rs:33-55,162-204) each re-derive min/max from pos+size element streams; `point_in_node_aabb` converts pos+size → min/max inline.
Verdict: mostly justified parallels (serde shape vs runtime shape vs hitbox bag), but the absence of any named AABB type/helpers in `baumhard::util::geometry` (only `aabb_center` exists) is why G1/G2 keep re-materializing. A tiny `Aabb { min, max }` (or free fns `aabb_from_pos_size`, `aabb_union`) in baumhard would give conversions one home.
Effort: M (optional, do with G1)

Clean in sweep 3: `hit_test_edge` and edge-handle insertion properly reuse `connection::build_connection_path` / `distance_to_path` / `normal_at_t` (structural.rs:270-275 even documents the reuse rule); Bézier math exists only in `lib/baumhard/src/mindmap/connection/`; `point_inside_any_node`'s strict-with-epsilon semantics are a documented, justified divergence; maptool contains no geometry re-implementations (operates on serde values / typed model only).

## Sweep 4 — Text / grapheme discipline (§B3)

### T1. Character-insertion implemented three times; the text editor fixed the cursor-drift bug, console and label editors still have it
Severity: P2 | Category: correctness, duplication | Confidence: high
Files: src/application/app/text_edit/editor.rs:544-598 (correct); src/application/app/console_input/edit.rs:261-288 (`insert_text`, buggy pattern); src/application/app/label_edit.rs:31-55 (`route_label_edit_key`, buggy pattern) + src/application/app/text_edit/mod.rs:140-143 (`insert_at_cursor`, the `cursor + 1` primitive both buggy sites use)
Evidence: editor.rs's comment names the exact failure: an IME delivering `"한"` (three jamo, one cluster) or dead-key `"e\u{0301}"` "would otherwise call `insert_at_cursor` once per char and increment `cursor` by `+1` per char — but `count_grapheme_clusters` of the resulting buffer collapses the codepoints into one cluster, leaving `cursor_grapheme_pos` past the buffer's grapheme count" — and fixes it with a pre/post cluster-count delta. console `insert_text` does exactly the warned-against loop: `for ch in text.chars() { ... insert_str_at_grapheme(input, *cursor, encoded); *cursor += 1; }`. label_edit does the same via `insert_at_cursor` per char — its comment even says "payloads can carry IME / dead-key multi-char sequences, so iterate". console/tests/grapheme.rs:25-40 locks in the per-char+`+=1` behavior for ASCII only.
Why it matters: §B3 + §2 "unify the shapes": same primitive, three shapes, two of which corrupt the grapheme cursor on multi-codepoint payloads (cursor lands past end / right of the intended position in console and label/portal-text editors).
Fix: add a baumhard primitive `grapheme_chad::insert_str_at_grapheme_counted(buffer, cursor, s) -> usize` returning the cluster-count delta (the pre/post-count logic from editor.rs moves into it, one count walk saved); all three editors call it. Ship with combining-mark/jamo/ZWJ `do_*()` tests per §B3.
Effort: M

### T2. `split_section` re-implements two grapheme_chad primitives via direct unicode-segmentation
Severity: P2 | Category: duplication, convention | Confidence: high
Files: src/application/document/nodes/section_structure.rs:311,332,344-354
Evidence: `original_text.graphemes(true).count()` (≡ `count_grapheme_clusters`) and `original_text.grapheme_indices(true).nth(split_grapheme).map(|(b,_)| b)` (≡ `find_byte_index_of_grapheme`) — both canonical functions exist in `baumhard::util::grapheme_chad` and are imported elsewhere in the same crate.
Why it matters: §B3 "All text primitives live in grapheme_chad.rs ... call a function from that file".
Fix: replace with `count_grapheme_clusters` / `find_byte_index_of_grapheme`; drop the local `use unicode_segmentation`.
Effort: S

### T3. Word/token/line-boundary scans hand-rolled in the app crate (missing grapheme_chad primitives), with an internal near-duplicate pair
Severity: P2 | Category: duplication, api-design | Confidence: high
Files: src/application/app/console_input/edit.rs:169-195 (`kill_word` backward word scan); src/application/app/console_input/completion.rs:107-127 (`accept_console_completion` backward token scan — same skip-non-whitespace loop, lines 113-116 ≅ edit.rs:184-186); src/application/app/text_edit/mod.rs:166-197 (`cursor_to_line_start` / `cursor_to_line_end` grapheme line-boundary walks); src/application/console/commands/section/mod.rs:180,196-199,464-468 (20-cluster preview clip + per-grapheme walk)
Evidence: all use `unicode_segmentation` directly from the app crate; the backward "step over whitespace clusters, then over non-whitespace clusters" loop exists twice with cosmetic differences; the line-boundary walks re-derive what `grapheme_chad`'s line-oriented helpers (`find_nth_line_grapheme_range`, `count_number_lines`) are the designated home for.
Why it matters: §1 "Missing primitives are added to Baumhard, not to src/application/"; §B3 "New text primitives go in grapheme_chad.rs" (with `do_*()` test + bench in the same commit). Word-jump Actions (`TextEditWordLeft/Right`, `ConsoleKillWord`) make word-boundary logic load-bearing on two surfaces already.
Fix: add to grapheme_chad: `prev_word_boundary(s, g_idx) -> usize`, `line_bounds_at(s, g_idx) -> (usize, usize)`, `clip_to_n_clusters(s, n) -> &str` (or equivalent); refactor the five call sites; the mandala crate's direct `unicode-segmentation` dependency can then be dropped (see D1).
Effort: M

### T4. Baumhard-internal grapheme duplicates next door to grapheme_chad
Severity: P3 | Category: duplication | Confidence: high
Files: lib/baumhard/src/mindmap/border.rs:1508-1510; lib/baumhard/src/mindmap/border_pattern.rs:315-321
Evidence: `pub(crate) fn count_clusters(s: &str) -> usize { s.graphemes(true).count() }` is byte-for-byte `grapheme_chad::count_grapheme_clusters` (grapheme_chad.rs:262); border_pattern's `fn clusters(s) -> Vec<String>` is a third private cluster-splitter.
Fix: import `count_grapheme_clusters`; if the owned-cluster-vec shape is genuinely needed, add it to grapheme_chad with test+bench per §B3.
Effort: S

Judged legit (not violations): whitespace/hex/control **per-char property checks** — console/commands/font.rs:152,984 (`chars().any(char::is_whitespace)` for quoting), console/completion.rs:88,238 (last-char whitespace), color_value.rs:39 & view.rs:600 (`is_ascii_hexdigit`), event_keyboard.rs:300 & text_edit/editor.rs:564 (`is_control` filtering), dispatch/native.rs:57-68 (`quote_console_arg` escaping — ASCII delimiters), console/parser.rs:46 (tokenizer splits only at ASCII quotes/spaces); `sections.truncate(1)` sites are `Vec::truncate`, not string truncation; console_input/edit.rs:181-186 and completion.rs:113-116 correctly *index by cluster* and only test char properties inside a cluster (the finding there is duplication, not byte-slicing). maptool: no user-string char/byte manipulation (export.rs:174 `&out[..40.min(len)]` slices generated JSON for an error preview — byte-safe risk is nil but `floor_char_boundary`-style care wouldn't hurt; not flagged).

## Sweep 5 — cosmic-text containment (§1 / §B5)

Result: CLEAN at the code level — zero `use cosmic_text` / `cosmic_text::` code references anywhere in src/ (all six textual hits are doc comments). The renderer consumes cosmic-text exclusively through baumhard re-exports (`baumhard::font::{Attrs, Buffer, Color, ...}`, lib/baumhard/src/font/mod.rs:53-76) and bridges (`hex_to_cosmic_color`, `cosmic_color_from_rgba`/`to_rgba`, `RegionFamilies`). Even the non-renderer picker modules use only the `font/color.rs` bridge. One structural hole:

### D1. mandala crate declares dead direct dependencies — including `cosmic-text` itself — and a [build-dependencies] section with no build.rs
Severity: P2 | Category: convention, dead-code | Confidence: high (grep-verified; confirm with cargo-machete/udeps)
Files: /home/user/mandala/Cargo.toml:21 (cosmic-text), :24 (tinyvec), :28 (futures), :34 (serde-lexpr), :39 (syn), :44 (ttf-parser), :46-51 ([build-dependencies]: walkdir, path-slash, ttf-parser, regex, lazy_static)
Evidence: no `cosmic_text::`, `syn::`, `serde_lexpr`, `tinyvec`, `futures::` (excluding wasm_bindgen_futures) code references anywhere in src/; the crate has no build.rs (baumhard's font-scanning build.rs lives at lib/baumhard/build.rs and declares its own build-deps), so the root [build-dependencies] section is inert.
Why it matters: `cosmic-text` as a direct dep is an open door for silent §B5 violations — any app file can `use cosmic_text::...` today and the compiler will not object; deleting the dep makes the containment compiler-enforced (renderer keeps working via the baumhard re-exports). `syn` in [dependencies] of the app binary is also a substantial compile-time cost for nothing.
Fix: remove the six unused [dependencies] entries and the root [build-dependencies] section; after T1-T3 land, `unicode-segmentation` (line 16) can go too. Run `cargo machete` (or `cargo udeps`) + `./build.sh` to confirm.
Effort: S

Also noted: renderer bypass check came back clean — border/console text passes go through `create_border_buffer` with `baumhard::font::Attrs`, region-styled text through the walker uses the `RegionFamilies` bridge (per its docs), and the only raw `Attrs::new().color(...)` constructions are for renderer-owned chrome (mode-status line, FPS), which is the sanctioned renderer privilege.

## Sweep 6 — Parallel enums / sync-by-hand tables

### E1. Action ↔ KeybindConfig field/pairs table: ~120-row hand-maintained mapping with no structural enforcement — and it has already desynced once
Severity: P2 | Category: ssot, testing | Confidence: high
Files: src/application/keybinds/config.rs:519-641 (the `(Action::X, &self.x)` pairs table); src/application/keybinds/action/mod.rs:99-102 ("Add a new variant here, extend `KeybindConfig` with a matching field + default"); src/application/keybinds/tests.rs:638-641 (past bug: "pre-fix the Action variant existed and was dispatched but had no `KeybindConfig` field, so users could not bind a key")
Evidence: the classifier side is compiler-enforced (mandala_derive::ActionClassify + exhaustive matches over `ActionKind`, pinned by tests.rs:485-517), but nothing iterates `ActionKind::iter()` against the pairs table / serde fields; the `SetBorderPreview` regression proves the failure mode is real and silent.
Why it matters: this is the single-dispatch-funnel's user-visible face (§3): an Action missing from the table dispatches internally but is unbindable/unconfigurable from keybinds.json.
Fix (either): (a) an exhaustiveness test: `for kind in ActionKind::iter()` assert `kind` is covered by the pairs table ∪ an explicit allowlist of parametric/payload kinds (`set_border_preview`-style ParametricBinding fields, gesture-only kinds); or (b) stronger — extend the ActionClassify derive with `#[action(config_field = "undo")]` and generate the pairs list.
Effort: M

### E2. maptool `verify` shape vocabulary drifted from the format spec and the runtime parser — spec-valid "circle" is flagged as a violation
Severity: P2 | Category: correctness, ssot | Confidence: high
Files: crates/maptool/src/verify/enums.rs:9-16 (`SHAPES` lacks "circle"); format/enums.md:31-35 ("rectangle, rounded_rectangle, ellipse, **circle**, diamond, parallelogram, hexagon ... `"circle"` is accepted as an alias for `"ellipse"`"); lib/baumhard/src/gfx_structs/shape.rs:87-93 (runtime accepts "rectangle", "ellipse", "circle"; everything else falls back to rectangle)
Evidence: three vocabularies, three owners: the verifier omits the alias the spec explicitly blesses, so `maptool verify` reports a violation on a map the loader/renderer handle per spec. (The reverse direction — diamond/parallelogram/hexagon verifying clean but rendering as rectangles — is spec-documented fallback behavior, fine.)
Why it matters: `verify` is the format's enforcement tool; a false positive on a spec-blessed value teaches authors to distrust it.
Fix: export the vocabulary from baumhard (e.g. `pub const SHAPE_NAMES: &[&str]` + `NodeShape::parse` publicly documented) and have maptool consume it; minimum fix: add "circle" to `SHAPES` with a test pinning spec parity (parse format/enums.md's list is overkill; a shared const is the SSOT move).
Effort: S

### E3. MacroSource duplicates MutationSource (same four variants, same ordering contract, "Mirrors" comment)
Severity: P3 | Category: duplication | Confidence: med
Files: src/application/macros/mod.rs:25-65; src/application/document/mutations_loader/mod.rs:38-58 (whose header says "the precedence order ... Code mirrors the doc ... Changes to the set or order update all three sites in the same commit" — i.e. sync-by-discipline, doc'd)
Evidence: both are `{App, User, Map, Inline}` with "ascending precedence" semantics; MacroSource layers privilege methods on top.
Verdict: borderline justified parallel (different capability surfaces) but the *tier order* is one piece of knowledge in two enums + one doc. A shared `SourceTier` enum (baumhard or app-common) with macro privilege methods as an extension would collapse it; at minimum add a test asserting the two enums' variant order matches.
Effort: M (or S for the pinning test)

### E4. Border-preset hint table has a silent fallback; the rest of the preset pipeline is properly single-sourced
Severity: P3 | Category: ssot | Confidence: high
Files: src/application/console/commands/border/complete.rs:86-95 (`preset_hint` match with `_ => ""`); canonical: lib/baumhard/src/mindmap/border.rs:62-68 (glyph table), :1124 (`preset_glyph_set`), :1155 (`BORDER_PRESETS` **derived from the table** — exemplary), re-exported at src/application/console/commands/border/mod.rs:81 (`PRESETS = BORDER_PRESETS`)
Evidence: definition, parsing, cycling (`next_border_preset`), and completion candidates all flow from one table; only the human-readable hint strings live in a parallel match that yields `""` for a future preset (new preset appears in completion with an empty hint — cosmetic drift, silent).
Fix: fold the hint into the baumhard preset table (`(name, glyphs, blurb)`) or add a test asserting `preset_hint(p)` is non-empty for every `BORDER_PRESETS` entry.
Effort: S

Clean in sweep 6: console verb registry (`COMMANDS` slice carries name/aliases/summary/usage/complete/execute — help, completion, and dispatch all derive from the one registry; the partial name-pin test at commands/mod.rs:136-139 is redundant-but-harmless); `GfxElementType` ↔ `GfxElement` (compiler-enforced exhaustive `element_type()` match, element.rs:285-288); `PORTAL_GLYPH_PRESETS` single table (edge.rs:311); `MacroSource::allows_action` now rides the compiler-enforced `Action::is_destructive` classifier (the old hand denylist bug is documented and fixed).

## Sweep 7 — Serialization shapes

### S1. The size-capped file read and the layered source-fallback driver are each copy-pasted three times across the user-tier loaders
Severity: P3 | Category: duplication | Confidence: high
Files: desktop reads (stat → cap → read_to_string → parse, byte-identical error strings): src/application/document/mutations_loader/platform_desktop.rs:50-70, src/application/keybinds/platform_desktop.rs:25-40, src/application/macros/loader/platform_desktop.rs:40-62. Web drivers (query-param → cap → parse → localStorage → cap → parse → default): src/application/document/mutations_loader/platform_web.rs:26-50, src/application/keybinds/platform_web.rs:23-47, src/application/macros/loader/platform_web.rs:29-53. Desktop layered drivers (explicit > XDG > default): mutations_loader/platform_desktop.rs:22-48 vs keybinds/platform_desktop.rs:45-67.
Evidence: e.g. the exact string `"{} exceeds size cap ({} bytes > {} max); refusing to load"` appears in all three desktop loaders. The primitives *are* shared (user_config::{MAX_USER_PAYLOAD_BYTES, payload_within_cap, xdg_mandala_path, web_storage::*}) — only the composed logic is tripled, and each file's header says "Mirrors ..." (sync-by-comment).
Why it matters: §5 "the answer is never to copy"; §2 explicitly distinguishes idiom (good) from shape (smell) — three identical shapes is the smell. A fourth user-tier loader (themes? palettes?) will make it four.
Fix: add to user_config: `pub fn read_capped(path: &Path) -> Result<String, String>` (kills the clearest triplication) and, optionally, `pub fn load_web_layered<T>(param: &str, storage_key: &str, label: &str, parse: impl Fn(&str) -> Result<T, String>) -> Option<T>`; loaders keep their per-type parse fns.
Effort: M

Clean in sweep 7: no duplicated `default_true`/`default_zero`-style serde fns (every `default_*` is a distinct domain value, defined once next to its struct); `skip_serializing_if` uses std predicates plus three purpose-built ones (`is_zero_u32`, `is_default_position`, `ZoomVisibility::is_default`), each single-site; `parse_mutations_json` is explicitly shared by app bundle + user file + tests; all JSON parsing funnels through `baumhard::format::json::parse`.

## Sweep 8 — Platform pairs (inventory + drift risk)

Pairs found (src/): run_wasm/* event handlers vs native event handlers (drift risk HIGH by construction — wasm explicitly lacks the drag state machine, run_wasm/event_cursor_moved.rs:6-7 "WASM has no hover / drag state today (full drag machine deferred to a later parity session)"; deep-dive owned by another agent — noted here only as the one pair with an acknowledged capability gap; `DRAG_THRESHOLD_SQ_PX` is cfg(not(wasm)) accordingly); clipboard.rs (clean module-level split; WASM read/write are logged stubs — a documented capability gap, not drift); now_ms (single cross-platform impl in common.rs:184 via web-time, comment at app/mod.rs:113 confirms single-source — clean); user_config (xdg.rs native-only + web_storage.rs wasm-only behind one mod — clean, complementary not duplicated); the three loader trios (drift-safe on primitives, tripled drivers → finding S1); keybinds resolution itself is fully shared (bind.rs/resolved.rs compile both sides — clean); mutations_loader (same shape as macros — S1); main.rs `parse_cli` native vs `?map=` query parsing in run_wasm (different input surfaces, no shared logic to drift — clean).
Shared-core hygiene is otherwise good: `is_double_click`, `compute_click_hit`, gesture math (touch_gesture.rs) and dispatch cores are written once and imported by both sides, exactly per §4/§T9.

## Sweep 9 — Logging idiom

### L1. Log-message prefix conventions are mixed; release builds compile ALL logging out
Severity: P3 | Category: convention, error-handling | Confidence: high
Files: prefix styles (sample): "macros: …" (macros/loader/*), "font::attrs: …" (baumhard font), "console history: …", bare "skipping invalid keybind '…'" (keybinds/config.rs:646), fn-name prefixes "dispatch_macro: …", "apply_set_border_preview: …". Cargo.toml:31: `log = { version = "0.4.29", features = ["release_max_level_off"] }`.
Evidence: three coexisting prefix idioms (module:, fn-name:, none). More consequentially, `release_max_level_off` means every `log::warn!`/`error!` — the §9-mandated failure channel for degraded frames — is compiled out of release binaries; the freeze watchdog already works around this by using `eprintln!` directly (freeze_watchdog.rs:142-158, justified). So in production the "degrade the frame, log, keep running" posture is actually "degrade silently".
Why it matters: §9 designates warn!/error! as the user-visible failure story; a convention that vanishes in release deserves an explicit note in CODE_CONVENTIONS (or a release feature that keeps warn+error, e.g. `release_max_level_warn`).
Fix: decide and document: either switch to `release_max_level_warn` (keeps the §9 channel alive at negligible cost) or record the deliberate silence in CODE_CONVENTIONS §9; separately, pick one prefix idiom ("<area>: message") and normalize on the way past (§5 drive-by rule).
Effort: S
Also: `println!` hits are confined to tests (gfx_structs/tests/tree_tests.rs:1051,1059,1133 — debug prints worth deleting on the way past), the generate_stress_map bin, and the watchdog's deliberate eprintln. No dbg!() anywhere. maptool prints are CLI output (fine).

## Sweep 10 — TODO/HACK/allow/unsafe/commented-out code

### A1. Crate-wide `#![allow(dead_code)]` on the entire mandala binary, unjustified
Severity: P2 | Category: dead-code, convention | Confidence: high
Files: src/main.rs:3
Evidence: `#![allow(dead_code)]` with no comment, suppressing dead-code detection across ~83K LOC — the compiler cannot flag any unused fn/struct/field in the whole app crate.
Why it matters: §5 "no dead code" is unenforceable while this stands; it also hides the very drift this review hunts (e.g. orphaned helpers after refactors). Baumhard notably does NOT do this — its allows are per-item and mostly seam-documented (tree.rs:129-157, scene.rs:39, model/line.rs:64-68).
Fix: delete the attribute, build, then either delete what falls out or justify survivors with targeted `#[allow(dead_code)]` + seam comments (per the tree.rs pattern).
Effort: M

### A2. #[allow] inventory (33 total) — the rest, judged
Severity: P3 | Category: convention | Confidence: high
Files/verdicts: `#[allow(unused_imports)]` on re-export hubs — keybinds/mod.rs:32,38,46,49,56, console/mod.rs:37, console/traits/mod.rs:35, document/mod.rs:83, renderer/mod.rs:57, mutator_builder/mod.rs:25 (10 sites; each hides a possibly-obsolete re-export — convert to `pub use` where intended for consumers, else delete); `clippy::too_many_arguments` ×5 (exec.rs:125,205; tree_builder/border.rs:322; scene_builder/builder.rs:218, connection.rs:62 — accepted, these are the documented wide-param builders); `#[allow(deprecated)]` app/mod.rs:555 (documented, winit WASM constructor — justified); `unreachable_patterns` console/commands/mutation.rs:363 (justified: non_exhaustive wildcard); `non_camel_case_types` mutator_builder/ast.rs:183 (serde-shape choice — acceptable, documented enum naming); baumhard seam allows tree.rs:129,132,135,137,157 (documented seams — but see D4: CONCEPTS claims these fields are "used narrowly today", the allows prove they are unused); test-local dead_code allows (fixtures) — fine.
Effort: S each, on the way past

Clean in sweep 10: ZERO `TODO`/`FIXME`/`HACK`/`XXX` comments in all three crates (§5 fully honored — remarkable at this size); zero `todo!()`/`unimplemented!()`; zero `unsafe` in baumhard (§B7 holds) AND zero in the app crate and maptool; no commented-out code blocks found (all `// let` / `// return` hits are prose).

## Sweep 11 — unwrap()/expect() posture (§9)

### U1. 26 bare `unwrap()` calls in production code (25 baumhard, 1 app, 0 maptool) — §9: "Bare unwrap() outside tests is a bug"
Severity: P2 (aggregate; individual risk varies) | Category: error-handling | Confidence: high
Counts: raw `.unwrap()` including tests: src 236, baumhard 70, maptool 75; outside test modules/files: baumhard 25, src 1, maptool 0.
The complete production list, classified (I = reachable from interactive paths after first frame):
1. lib/baumhard/src/gfx_structs/util/regions.rs:293 — `self.inner.write().unwrap()` — I; lock-poison panics, while `RegionError::Poisoned` exists precisely for this case elsewhere in the same layer → inconsistent posture, worst offender.
2. lib/baumhard/src/util/grapheme_chad.rs:63 — `find_byte_index_of_grapheme(...).unwrap()` — I (every text edit); invariant provable locally but line 47 tolerates OOB with `unwrap_or`, line 63 doesn't — asymmetric.
3-4. lib/baumhard/src/gfx_structs/tree_walker.rs:52,540 — option unwraps on the hot walk — I.
5. lib/baumhard/src/font/fonts.rs:384 — `FONT_SOURCES.get(name).unwrap()` — I; documented "Panics if name is not in FONT_SOURCES" (enum-keyed map, invariant-safe today; should be `expect("AppFont variant missing from FONT_SOURCES")`).
6. lib/baumhard/src/font/fonts.rs:391 — `choose(&mut rng).unwrap()` — see U2.
7-8. lib/baumhard/src/mindmap/connection/bezier.rs:74,85 — `last().unwrap()` on arc-length tables non-empty by construction — I (scene build).
9. lib/baumhard/src/mindmap/scene_builder/connection.rs:372 — `samples.last().unwrap()` — I.
10. lib/baumhard/src/mindmap/scene_builder/label.rs:139 — `label_edit_override.unwrap()` — I.
11-18. lib/baumhard/src/gfx_structs/model/matrix.rs:33,57,75,92 and model/line.rs:31,37,123,163 — get-after-auto-expand invariant unwraps — I (mutation path).
19-21. lib/baumhard/src/gfx_structs/model/line.rs:318,332,349,372 — same family — I.
22-23. lib/baumhard/src/gfx_structs/util/region_indexer.rs:92,103 — get_mut-after-ensure — I.
24. lib/baumhard/src/core/primitives.rs:379 — `self.regions.get(region).unwrap()` — I.
25. src/application/console/commands/canvas.rs:185 — `verb.unwrap()` inside a `Some("preset")|Some("color")|...` match arm — I (tab completion); zero-risk but trivially removable by binding `Some(v)` in the pattern.
26. (startup-adjacent) none — all remaining expect() sites carry messages.
Why it matters: §9's letter is absolute; §B0 requires baumhard primitives "panic-free in interactive paths". Most of these are invariant-guarded, but each is one refactor away from a mid-edit crash, and the crate-wide posture (documented `RegionError`, defensive `let-else` idiom elsewhere) shows the intended alternative exists.
Fix: mechanical pass — pattern-bind instead of unwrap where the match already guarantees (canvas.rs:185, tree_walker.rs:52,540, label.rs:139), `expect("<invariant>")` where a local invariant is real (matrix/line get-after-ensure, fonts.rs:384, bezier last()), degrade-and-log for regions.rs:293 (map poison to `RegionError::Poisoned` like its siblings) and grapheme_chad.rs:63 (`unwrap_or(end_of_target_line_idx)`).
Effort: M

### U2. `get_some_font` — "test-only" rand-based helper compiled into the production pub surface
Severity: P3 | Category: api-design | Confidence: high
Files: lib/baumhard/src/font/fonts.rs:387-392
Evidence: "Pick a random compiled-in font source. **Test-only helper** — production paths should pick fonts deterministically." — but it is `pub`, not `#[cfg(test)]`-gated (can't be, `pub mod tests` benches may need it), and drags `rand` into the release build.
Fix: move it into the `font/tests` tree (reachable by the bench harness per §T2.2) or feature-gate; §B9-doc the cost either way.
Effort: S

expect() posture (sampled, src/): startup expects all carry human-readable messages per §9 (pipeline.rs:47,59; run_native.rs:59,99; run_wasm/mod.rs:507-524; mutations_loader/builtin.rs:24 — build-time-invariant bundle parses correctly use expect). Interactive-path `expect("just checked")` invariant style appears ~15× (document/nodes/section_text.rs ×9, section_structure.rs:56-85, nodes/border.rs:912, edges/closure_helpers.rs:30, click.rs:224,234, console_input/completion.rs:122, lifecycle.rs:392): tension with §9's "interactive paths must not panic", but each is locally provable and message-carrying — recorded as accepted idiom, not findings.

## Sweep 12 — British spellings (project mandates American, CLAUDE.md §6)

### B1. Code identifiers with British spellings (each a finding per instructions)
Severity: P3 | Category: convention | Confidence: high
Files (production identifiers): src/application/console/commands/color.rs:330 `fn apply_section_colours(` (production fn name); src/application/app/run_native.rs:329 `let recognised`; src/application/app/run_wasm/event_touch.rs:57 `let recognised`; benchmark IDs lib/baumhard/benches/test_bench.rs:203 `"shape_ellipse_contains_centre_and_rim"` and :264 `"region_indexer_initialise"` — the latter cited by name in lib/baumhard/CONVENTIONS.md §B6, so renaming is a bench+tests+doc change per §T6.
Files (test/do_* identifiers — rename with their benches where paired): lib/baumhard/src/gfx_structs/tests/shape_tests.rs:76,80 `test/do_shape_ellipse_contains_centre_and_rim`; src/application/app/throttled_interaction/{moving_node.rs:167, moving_section.rs:147, edge_handle.rs:139, edge_label.rs:109, section_resize.rs:171, portal_label.rs:109, node_resize.rs:142} `test_new_initialises_*`; src/application/app/dispatch/cross_dispatch/camera.rs:217,220 (`..._centre_and_selects_it`, `let centre`); src/application/document/tests_hit_move.rs:1041,1045,1070,1099,1105 (centre); src/application/color_picker_overlay/.../dynamic_context.rs:554 (`crosshair_centre_...`); lib/baumhard/src/mindmap/scene_builder/section_resize_handle.rs:397 (`..._centre_rounds_south_east`); lib/baumhard/src/mindmap/border_pattern.rs:396 (`parse_unrecognised_...`); lib/baumhard/src/bin/generate_stress_map.rs:659 (`..._serialises_...`).
Fix: rename (American) — for the two bench IDs update benches/test_bench.rs + the `do_*` fns + CONVENTIONS §B6 in one commit per §T6.
Effort: M (mechanical but wide)

### B2. British spellings in code comments: ~577 occurrences across src/ + lib/ (aggregate finding)
Severity: P3 | Category: convention | Confidence: high
Counts (word → occurrences in .rs files): colour 233, centre 110, behaviour 96, recognise* 60, honour* 30, serialise* 28, initialise* 15, quantis* 12, normalise* 10, grey 7, catalogue 3, licence 1, synchronis* 1. Heaviest in baumhard doc comments (colour/centre) and touch_gesture.rs (recogniser).
Fix: mechanical sweep (word-boundary sed with review); §2's "honor the idioms" — the codebase's own convention docs already use American in headings.
Effort: M

### B3. Docs: British spellings widespread in CONCEPTS.md and format/sections.md (one docs finding as instructed)
Severity: P3 | Category: docs | Confidence: high
Files (count of matches): CONCEPTS.md 47; work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md 37; format/sections.md 13; format/border-patterns.md 3; lib/baumhard/CONVENTIONS.md 2 ("serialises", "region_indexer_initialise" — the latter is the bench name, see B1); format/canvas.md 2; schema.md/portal-labels.md/mutations.md/macros.md 1 each.
Effort: S

## Cross-cutting docs-vs-code drift (found during sweeps)

### D2. CODE_CONVENTIONS.md §3 macro-privilege paragraph contradicts the code on whether the gates are live
Severity: P2 | Category: docs | Confidence: high
Files: CODE_CONVENTIONS.md ("Today only the User tier loads, so the gates are dormant; they MUST hold before app-bundle / map-inline / node-inline tiers ship."); src/application/macros/mod.rs:36-37 ("All four tiers load today on native; the gate is fully active.") and :85-86 (same statement in `allows_action`).
Why it matters: this is the security-model paragraph of the contract document; a reviewer consulting the conventions would wrongly conclude the App/Map/Inline loaders don't exist yet and skip gate review on loader changes.
Fix: update CODE_CONVENTIONS §3 to the four-tiers-live reality (conventions change in response to code — §0 says the doc must track).
Effort: S

### D3. CONCEPTS.md documents the border default preset as "rounded"; the code default is "light" (with rationale)
Severity: P3 | Category: docs | Confidence: high
Files: CONCEPTS.md §3 Border geometry ("`"rounded"` (`─ │ ╭ ╮ ╰ ╯`, the default)"); lib/baumhard/src/mindmap/model/node.rs:503-509 (`default_border_preset() -> "light"`, comment explains why light replaced rounded: corner glyphs join cleanly).
Fix: update CONCEPTS.md (and check format/schema.md's default claim).
Effort: S

### D4. CONCEPTS.md says `Tree.position` / `pending_mutations` are "used narrowly today"; the code marks both `#[allow(dead_code)]` (unused)
Severity: P3 | Category: docs | Confidence: med
Files: CONCEPTS.md `Tree<T, M>` entry; lib/baumhard/src/gfx_structs/tree.rs:129-133.
Fix: reword CONCEPTS to "reserved seams, currently unused" (matches the allows), or wire the fields and drop the allows.
Effort: S

## Sweeps that came back CLEAN (summary)
- Sweep 5 (cosmic-text containment): zero code-level violations in src/; renderer uses baumhard bridges/re-exports exclusively (only the dead Cargo dep, D1).
- Sweep 10 (TODO/FIXME/HACK/todo!/unimplemented!/unsafe/commented-out code): completely clean across all three crates except the two `allow` findings (A1, A2).
- Zoom limits, throttle budget, double-click window, border geometry fractions, picker cell tables (Sweep 1 sub-checks): single-sourced.
- Color conversion math itself (Sweep 2): single-sourced with delegating bridges; maptool color-free.
- Bézier/path math (Sweep 3): single home in baumhard::mindmap::connection, correctly reused by app hit-testing and edge mutation.
- Serde default fns and skip predicates (Sweep 7): no copy-paste helpers.
- now_ms, clipboard, user_config primitives (Sweep 8): single-sourced/cleanly split.
- maptool unwrap posture (Sweep 11): zero production unwraps.
- Console verb registry, GfxElementType, BORDER_PRESETS, PORTAL_GLYPH_PRESETS (Sweep 6): structurally enforced or single-table.
