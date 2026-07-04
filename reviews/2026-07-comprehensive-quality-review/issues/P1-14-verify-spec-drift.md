# P1-14: `maptool verify` vs format spec vs runtime drift — "circle" falsely rejected, duplicate-edge check missing, channel-collision severity wrong, validation.md stale

**Severity:** P1 (the format's enforcement tool contradicts the spec it enforces) · **Area:** maptool/verify + format docs + baumhard model

## Problems (each independently verified)

1. **`"circle"` falsely rejected** (reproduced): `verify/enums.rs:9-16` `SHAPES` lacks `"circle"`; `format/enums.md:27-45` lists it as known ("the convenience spelling"); the runtime accepts it case-insensitively as Ellipse (`lib/baumhard/src/gfx_structs/shape.rs:87-93`). `shape:"circle"` → verify exit 1 on a file the spec blesses and the app renders. Also: shape matching is case-insensitive at runtime but case-sensitive in verify (`"Rectangle"` renders, fails verify).

2. **Duplicate `(from_id, to_id, edge_type)` tuples never checked**: CONCEPTS §3 declares duplicates "a duplicate and a validation error", and the entire edge-identity architecture (EdgeRef lookups, scene-cache keys, renderer buffer/hitbox maps) rests on tuple uniqueness — a duplicate makes `SceneConnectionCache::insert` overwrite so both edges render the second one's geometry, and every EdgeRef lookup binds to the first match. No verify module, no loader warning, no format/validation.md entry exists.

3. **Channel-collision severity**: `format/validation.md:96-100` documents the section-channel-collision check as a *warning* authors "can ignore" (broadcast is legitimate); the implementation is a hard violation → exit 1 (`verify/sections.rs:124-151`; `Violation` has no severity tier, `main.rs:235-255` exits 1 on any). The documented CI recipe cannot pass for legitimate broadcast maps.

4. **validation.md stale in four more places**: says text-run `end` is checked against "code-point count" — the check is grapheme clusters (`verify/text_runs.rs:3-5,42`, whose own tests pin the unit migration); says AABB check applies "when `size` is set" — fill-parent AABBs are checked too (sections.rs:44-45); the 1024 section cap is enforced but undocumented; the non-finite zoom-bounds checks (zoom_bounds.rs:78-95) are undocumented.

5. **Node-size ceiling asymmetry**: the app enforces `MAX_NODE_AXIS = 1_000_000.0` on every setter (`src/application/document/nodes/mod.rs:854-870`); verify has no node ceiling — `width: 1e30` passes verify while the app refuses to produce it.

## Fix plan

1. Export canonical vocabularies from baumhard and consume them in verify — `pub const KNOWN_SHAPES` beside `NodeShape` (include "circle"; match case-insensitively or normalize), and reuse the existing `DISPLAY_MODE_LINE/PORTAL` consts verify currently shadows with string literals. Single source of truth per CODE_CONVENTIONS §5.
2. Add `verify/edges.rs`: HashSet over `(from_id, to_id, edge_type)`, violation naming both indices; list it in validation.md. Add a loader-side `log::warn!` on collision so hand-edited maps degrade loudly at runtime too.
3. Add a `Warning` tier to `Violation` (print, exit 0 when warnings-only) and downgrade channel-collision to it — matching the documented intent. Update `main.rs` exit-code logic + tests.
4. Add the node-size ceiling to verify (share the constant — see P1-25 validation-SSOT issue) and document it.
5. One validation.md sweep closing items 3–5 (grapheme wording, fill-parent, caps, zoom checks).

## Acceptance criteria

- `shape:"circle"` verifies clean; a duplicate-tuple map is flagged; a broadcast-channel map exits 0 with a printed warning.
- validation.md lists exactly the implemented checks with correct semantics.
- `./test.sh` green (maptool has 130 tests; extend, don't regress).

## Pointers

`crates/maptool/src/verify/{enums.rs,sections.rs,references.rs,zoom_bounds.rs,mod.rs}`, `crates/maptool/src/main.rs:235-255`; `lib/baumhard/src/gfx_structs/shape.rs`; `format/{validation.md,enums.md}`; CONCEPTS §3 (MindEdge).
