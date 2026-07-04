# P2-46: Constants and micro-geometry SSOT — drag threshold drifted (4px vs 5px), selection cyan defined four ways, 14pt/1.2 sizing tripled, point-in-AABB tripled

**Severity:** P2 (small knobs, real drift already present) · **Area:** cross-crate

## Problems (all sites verified)

1. **Pointer drag threshold drifted**: `touch_gesture.rs:79-84` claims "Mirrors the existing mouse drag threshold … same value, same intent" then defines `MOVE_THRESHOLD_PX = 4.0`; the mouse constant is `DRAG_THRESHOLD_SQ_PX = 25.0` (= 5px linear), in a different file than the comment claims, under a different name. **Fix:** one `POINTER_DRAG_THRESHOLD_PX: f64 = 5.0`; derive the squared form; touch uses the squared compare too (currently `.sqrt()` per move). If 4px-for-touch is intentional, say so and why.
2. **Selection-highlight cyan ×4, three byte-values, two crates**: baumhard `SELECTION_HIGHLIGHT_HEX = "#00E5FF"` (pub(crate) — *forcing* the app to duplicate), app `HIGHLIGHT_COLOR: [f32;4] = [0.0, 0.9, 1.0, 1.0]` (=229.5), renderer `Color::rgba(0,230,255,255)` and `Color::rgba(0,229,255,255)` (`mindmap/mod.rs:51`; `document/types.rs:15`; `renderer/mod.rs:911-916`; `scene_buffers.rs:101-102`). §1: color is Baumhard's. **Fix:** one `pub` canonical `SELECTION_HIGHLIGHT: Rgba` in baumhard; derive hex/float/cosmic forms via the existing conversion fns; replace all four.
3. **Default text size 14pt + line-height 1.2 tripled**: `tree_builder/node.rs:73,141-147` vs `document/mod.rs:268-276` (identical effective-scale block, "Mirrors …" comment) vs `custom/sync.rs:32-34`. If baumhard changes 1.2 → 1.25, the app's auto-size measurement drifts from the render and auto-sized nodes clip. **Fix:** `pub const DEFAULT_TEXT_SIZE_PT`, `LINE_HEIGHT_FACTOR`, and a shared `effective_section_scale(&MindSection)` in baumhard; call from all three.
4. **Point-in-AABB (closed interval) ×3**: `renderer/hit.rs:17-19` (whose own test comment concedes the consolidation is local-only), `document/hit_test.rs:178-187` (two inline copies), `scene.rs:93` (private). **Fix:** `pub fn aabb_contains(p, min, max)` in `baumhard::util::geometry` with `do_*` test + bench; replace the three sites (the deliberately-strict epsilon-inset variant in connection.rs stays, documented).
5. **Node-center by hand ×4** despite `aabb_center`/`MindNode::center_vec2` existing: `scene_builder/edge_handle.rs:85-86`, `portal_geometry.rs:119`, `app/edge_drag.rs:49-50`, `document/edges/structural.rs:277`. **Fix:** mechanical replacement.
6. **Camera fit padding 0.05 ×2** (`renderer/hit.rs:53,193`) with a comment anticipating a third caller. **Fix:** one const (note: `fit_camera_to_scene` is dead per P2-41 — one site may just be deleted).
7. **Epsilon deviations**: `custom/sync.rs:260-261` compares node sizes with `> f32::EPSILON` — effectively `!=` at magnitude 100+ (below one ULP at 256.0); either an accidental exact-compare or a misspelled `pretty_inequal`. Baumhard has no f64 `almost_equal`, so app tests hand-roll `1e-6` tolerances on f64 positions. **Fix:** sync.rs → `pretty_inequal` (or document exact-inequality intent); add `almost_equal_f64` (+do_* test + bench) and migrate the test hand-rolls (§5 drive-by).
8. **`is_prime` ceiling can defeat the RegionParams prime guard**: `is_prime(n)` returns `false` for every n > 10_000 (documented), and `RegionParams` asserts `!is_prime(resolution)` — a 10007px canvas dimension sails through into the degenerate-grid case the table exists to prevent. **Fix:** trial-division fallback above the ceiling (√n ≈ 100 iterations at construction frequency — trivial), or assert `n <= PRIME_CEILING` so the limit is loud.

## Acceptance criteria

- Each knob exists exactly once (grep per constant); the four cyan sites render identical bytes.
- Threshold parity (or documented divergence) between mouse and touch.
- `./test.sh` green; new geometry primitives benched (§B3-style same-commit).

## Pointers

CODE_CONVENTIONS §1 (color/geometry are Baumhard's), §5; crosscutting findings file (sweeps 1-3) for full evidence.
