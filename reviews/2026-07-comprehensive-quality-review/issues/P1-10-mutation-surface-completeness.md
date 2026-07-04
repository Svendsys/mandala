# P1-10: Mutation-surface completeness — model GlyphLine/GlyphLines deltas silently ignored; ChangeRegionRange panics; ApplyOperation per-field semantics undefined or lossy

**Severity:** P1 (three related defects in the mutation contract) · **Area:** baumhard/gfx_structs

## Problem A — `DeltaGlyphModel` carrying `GlyphLine` / `GlyphLines` does nothing

`lib/baumhard/src/gfx_structs/model/glyph_model.rs:117-131` (`apply_operation`) reads only `delta.position()`, `delta.layer()`, `delta.glyph_matrix()`. The accessors `delta.glyph_line()` / `delta.glyph_lines()` (`model/mutator.rs:134-152`) have **zero callers** repo-wide. A serde-authorable `Mutation::ModelDelta` carrying `GlyphModelField::GlyphLine(3, line)` walks the tree, matches its channel, and mutates nothing — silently — though the variant docs promise "Replace one line at `line_num`". CONVENTIONS §B4 defines a new mutation as "a field variant plus a branch in `DeltaGlyphModel::apply_to`" — the branch is missing for two shipped variants. (The sanctioned silent-ignore covers *type* mismatches, not advertised same-type variants.)

**Fix:** add the two branches (`operation.apply(matrix.ensure_line(n), line)` mirroring how tests hand-apply), or delete the variants per §10. Ship `do_*` tests through `apply_to`, not hand-application.

## Problem B — `GlyphAreaCommand::ChangeRegionRange` panics on a missing region, reachable from user JSON

`lib/baumhard/src/gfx_structs/area.rs:417-424`:

```rust
let mut current = *self.regions.get(*current_range).expect("No region found");
```

Dispatched unconditionally from `Applicable<GlyphArea> for GlyphAreaCommand` (`area_mutators.rs:175-177`). `GlyphAreaCommand` is serde-deserializable and rides `Mutation::AreaCommand` in user/map/inline custom-mutation JSON, which the dispatcher applies to live elements (`src/application/document/custom/mod.rs:292-316`). A mutation authored with a stale range panics the editor mid-edit — a §9 violation. `submit_region` (`core/primitives.rs:182-194`) was already converted from panic to warn-and-drop for exactly this reason.

**Fix:** `let Some(current) = self.regions.get(*current_range) else { log::warn!(...); return; };` mirroring `submit_region`; update the `# Panics` doc; regression test.

## Problem C — `ApplyOperation` semantics are field-dependent and silently lossy

`lib/baumhard/src/core/primitives.rs:442-483`; `area.rs:301-350`; `model/matrix.rs:46-96`; `model/line.rs:53-58`; `model/glyph_model.rs:124-126`:

- `Delete` ("Reset to default") works on numeric fields via `T::default()` but on `Text` and `ColorFontRegions` falls through `_ => {}` — silent no-op contradicting the enum's own doc.
- `Multiply` on Text/Regions silently no-ops (not even logged) while on Position/Bounds it does component-wise multiply.
- The `AddAssign+SubAssign+MulAssign+Default` bound forces invented arithmetic on matrix/line/component: `GlyphLine::add_assign` deliberately performs *Assign* ("Using `GlyphLineOp::Assign` here intentionally"), so `ApplyOperation::Add` on a matrix means per-line **overwrite**; `MulAssign` semantics are improvised (matrix.rs:63-66 comment: "wtf does it mean to multiply two glyphmatrices").
- `operation.apply(&mut self.layer, delta_layer)` with `Subtract` **underflows `usize`** (debug panic / release wraparound) when delta > layer.

**Fix:** extend the `apply_overwrite_or_reset` pattern (already used for Outline/Shape/ZoomVisibility on the area side) to Text/Regions/Matrix: define Delete explicitly (clear) or log-and-ignore explicitly; `log::warn!` on unsupported ops; `saturating_sub` for layer; document a per-field operation table on `GlyphAreaField`/`GlyphModelField` (§B9). Consider dropping the fake `MulAssign`/`SubAssign` impls in favor of explicit per-op arms — the trait bound is the wrong seam for non-numeric fields.

## Acceptance criteria

- Every `(field, operation)` pair either has defined behavior or produces one `log::warn!` — no silent no-ops, no panics, no underflow.
- Model GlyphLine/GlyphLines deltas apply through `apply_to` with tests, or the variants are gone.
- `./test.sh` green; new tests fail on current main for A, B, and the layer underflow.

## Pointers

CONVENTIONS §B4 (mutation surface), §B9 (doc costs); CODE_CONVENTIONS §9, §10; existing exemplar: `GlyphArea::apply_overwrite_or_reset` (`area.rs:480-499`).
