# P1-21: Broken dormant baumhard primitives — `split_and_separate` emits inverted ranges, `GlyphArea::rotate` doesn't rotate around the pivot, `GlyphLine` ignore_initial_space mixes byte/char/grapheme indices

**Severity:** P1 (pub primitives that compute wrong results; all currently unused or serde-only-reachable — fix or delete per §10) · **Area:** baumhard core/gfx_structs

## Problem A — `ColorFontRegions::split_and_separate` produces inverted/empty ranges

`lib/baumhard/src/core/primitives.rs:196-220`. For region `[5,10)` with `range=[3,7)`: the overlap branch sets left half to `[5,3)` (INVERTED — `magnitude()` will underflow later) and right half to `[7,14)`; the correct result for a region right of the insertion point is a pure shift `[9,14)`. A region exactly equal to `range` leaves an empty `[start,start)` husk. Tests only cover regions starting before `range.start`. Zero production callers (live insertion path is `insert_regions_at`/`shift_regions_after`) — but it is a benched, documented, pub primitive that CONCEPTS names among the five consistency primitives; exactly what a plugin will reach for.

**Fix:** restrict the split branch to true straddlers (`region.start < range.start && region.end > range.start`); shift regions with `start >= range.start` wholesale; add the missing truth-table cases — or delete the primitive per §10. No half-state.

## Problem B — `GlyphArea::rotate` never translates back

`lib/baumhard/src/gfx_structs/area.rs:471-478`:

```rust
self.position = OrderedVec2::from_vec2(Vec2::from_angle(angle).rotate(self.position.to_vec2() - pivot));
```

Missing `+ pivot` — every call teleports the area toward the origin. Its two siblings (`GfxElement::rotate`, `GlyphModel::rotate`) correctly use `clockwise_rotation_around_pivot` and take **degrees clockwise**; this one takes radians counterclockwise. Zero callers, zero tests.

**Fix:** reimplement via `clockwise_rotation_around_pivot(position, pivot, degrees)` for sibling parity (+doc), or delete. Ship a `do_*` test either way.

## Problem C — `GlyphLine::perform_op` ignore_initial_space path

`lib/baumhard/src/gfx_structs/model/line.rs:113-168` + `component.rs:157-163`:
1. `index_of_first_non_space_char` returns a **char ordinal**; `perform_op` feeds it to `String::split_off` (**byte** index) → panic on multi-byte leading whitespace (U+3000 ideographic space); the same value is then reused as a **grapheme** offset (`overriding_insert`). Three index units in four lines (§B3 violation).
2. `SubAssign`/`MulAssign` arms index `self.line[i]` unguarded while the guarded arm (`AddAssign`) is dead code — rhs longer than lhs panics.
3. The trailing insert loop can panic when `begin_comp > 0` leaves `self.line.len() < i`.

No production constructor sets `ignore_initial_space = true`, but `GlyphLine` is serde-deserializable — a JSON `ModelDelta` GlyphMatrix payload can set it, and matrix `*Assign` ops then run this path from the mutation pipeline (§9 interactive).

**Fix:** route the split through grapheme_chad (locate first non-whitespace **grapheme**, `split_off_graphemes`); guard `self.line.get(i)` in all arms; delete or implement the dead `GlyphLineOp::AddAssign`/`Noop` arms; tests with U+3000 and rhs-longer shapes.

## Acceptance criteria

- Each primitive either computes documented-correct results with truth-table tests, or is deleted (§10 "delete rather than deprecate") with its bench entry removed in the same commit (§B8).
- No byte/char/grapheme unit mixing (all offsets via grapheme_chad).
- `./test.sh` green; `./test.sh --bench` green.

## Pointers

CONVENTIONS §B3, §B8; CODE_CONVENTIONS §5 ("hard parts are the work"), §10; `lib/baumhard/src/core/tests/primitives_tests.rs:32-59` (existing partial table).
