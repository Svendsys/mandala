# P2-42: §9 unwrap posture — 26 bare `unwrap()` in production code (complete inventory), one lock-poison panic with a purpose-built error variant sitting unused beside it

**Severity:** P2 (each is one refactor from a mid-edit crash; §9 calls bare unwrap outside tests a bug) · **Area:** both crates

## Problem

CODE_CONVENTIONS §9: "Bare `unwrap()` outside tests is a bug." Complete verified inventory (I = interactive-reachable after first frame):

**Worst offenders:**
1. `lib/baumhard/src/gfx_structs/util/regions.rs:293` — `inner.write().unwrap()` — I; lock-poison panic while `RegionError::Poisoned` exists **in the same layer for exactly this** — inconsistent posture.
2. `lib/baumhard/src/util/grapheme_chad.rs:63` — `find_byte_index_of_grapheme(...).unwrap()` — I (every text edit); the sibling lookup at :47 tolerates OOB via `unwrap_or`, this one doesn't.
3. `lib/baumhard/src/gfx_structs/tree_walker.rs:52,540` — option unwraps on the hot walk (1970s `is_some()/unwrap()` loops) — I.

**Invariant-guarded (convert to `expect("<invariant>")` or pattern-bind):**
4-5. `font/fonts.rs:384` (`FONT_SOURCES.get(name).unwrap()` — enum-keyed, safe today), :391 (`choose(..).unwrap()` — see get_some_font in P2-41).
6-8. `connection/bezier.rs:74,85`, `scene_builder/connection.rs:372` — `last().unwrap()` on tables non-empty by construction.
9. `scene_builder/label.rs:139` — `label_edit_override.unwrap()`.
10-21. `model/matrix.rs:33,57,75,92` + `model/line.rs:31,37,123,163,318,332,349,372` — get-after-auto-expand invariant unwraps (mutation path).
22-23. `gfx_structs/util/region_indexer.rs:92,103` — get_mut-after-ensure.
24. `core/primitives.rs:379` — `regions.get(region).unwrap()`.
25. `src/application/console/commands/canvas.rs:185` — `verb.unwrap()` inside a `Some("preset")|...` arm — trivially removable by binding `Some(v)`.

Also: four guarded `expect("guarded above")`-style calls in interactive paths (`click.rs:224,234`, `lifecycle.rs:392`, `completion.rs:122`) — dominated by explicit guards today; bind the value at the guard site so the re-unwrap disappears (house style is `let Some(..) else { return; }`).

## Fix plan

1. Degrade-and-log the two real hazards: regions.rs:293 maps poison → `RegionError::Poisoned`; grapheme_chad.rs:63 → `unwrap_or(end_of_line_idx)` matching its sibling.
2. Pattern-bind where the match already guarantees (canvas.rs:185, tree_walker 52/540 — modernize the two `is_some()/unwrap()` loops to `while let` in passing, per §5 drive-by).
3. Convert provable invariants to `expect("walker invariant: ...")`-style messages (matching the existing get_mutator/get_target idiom).
4. Optional enforcement: a clippy lint gate (`unwrap_used` at warn for non-test code) noted in `./test.sh --lint`'s advisory output.

## Acceptance criteria

- Zero bare `unwrap()` in non-test code across all three crates (grep with test-module exclusion).
- Behavior unchanged on valid inputs; the two degrade paths log and continue.
- `./test.sh` green.

## Pointers

CODE_CONVENTIONS §9; CONVENTIONS §B0 ("panic-free in interactive paths"); the crosscutting findings file for per-site reachability judgments.
