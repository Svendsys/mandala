# P1-24: Area-side and model-side mutation surfaces are hand-maintained structural twins — replace hand-written discriminants with strum, unify delta plumbing, fix accessor asymmetry

**Severity:** P1 (permanent upkeep tax on THE extension surface; drift already shipped) · **Area:** baumhard/gfx_structs

## Problem

Five parallel pairs are duplicated by hand between the `GlyphArea` and `GlyphModel` mutation surfaces:

1. **Discriminant enums + `variant()` maps** — `GlyphAreaFieldType` (`area_fields.rs:104,314`), `GlyphModelFieldType` (`model/mutator.rs:27,72`), `GlyphAreaCommandType` (`area_mutators.rs:29,119`), `GlyphModelCommandType` (`model/mutator.rs:188,256`), plus `GfxElementType`, `MutatorType`, `MutationType` — all exactly what strum's `EnumDiscriminants` derive generates, and strum is already a dependency of these files (`EnumIter`, `Display`).
2. **Delta wrappers** — `DeltaGlyphArea` vs `DeltaGlyphModel`: byte-similar `fields: FxHashMap<...>`, identical `new(Vec<Field>)` loops (`area_mutators.rs:251-258` vs `model/mutator.rs:107-113`), identical `operation_variant()` with Noop default, per-field `if let ... get(&Type::X)` accessor ladders.
3. **`same_type()` duplicated verbatim three times** (`area_fields.rs:332`, `model/mutator.rs:86`, `model/mutator.rs:272`).
4. **`apply_operation` bodies** share the read-accessor → `operation.apply` shape (`area.rs:278` vs `glyph_model.rs:117`).
5. **Accessor asymmetry with real cost**: area-side borrows (`text_ref() -> Option<&str>`, `color_font_regions() -> Option<&...>`) but model-side **clones** (`glyph_matrix() -> Option<GlyphMatrix>` deep-clones the whole matrix on EVERY apply, even when `operation_variant()` is Noop/Delete — `model/mutator.rs:116-123`; §B7 names `DeltaGlyphModel::apply_to` a hot loop by name).

The twin discipline has already broken: model GlyphLine/GlyphLines deltas are silently ignored (P1-10-A), Delete/Multiply semantics diverge per field (P1-10-C), and the accessor asymmetry above. Every new field variant currently needs 4–6 hand edits per side with no compiler help (CONVENTIONS §B4 names these enums as *the* mutation surface).

## Fix plan

1. Replace the seven hand-written discriminant enums + `variant()` fns with `#[derive(EnumDiscriminants)]` (`#[strum_discriminants(name(GlyphAreaFieldType))]` etc.). Zero seam change — the names stay.
2. Extract shared delta plumbing: a small generic `Delta<FT: Hash+Eq, F>` (or a local macro) providing `new()`/`operation_variant()`/storage once. Keep `GlyphAreaField`/`GlyphModelField` as separate public enums — they ARE the seam (§B4); only the plumbing unifies.
3. One `same_type` via the derived discriminants (`FieldType::from(a) == FieldType::from(b)`).
4. Make model accessors borrow (`glyph_matrix() -> Option<&GlyphMatrix>`); clone only inside the Add/Assign arms that need ownership. Fold in the rhs-by-value cleanups: `GlyphMatrix::{add,sub,mul}_assign` clone every rhs line despite owning rhs — consume via `into_iter()`; `GlyphComponent::add_assign` should `push_str`, not `self.text.clone() + &rhs`.
5. Run `./test.sh --bench` before/after; `DeltaGlyphModel` apply benches must not regress (they should improve from the clone removal).

## Acceptance criteria

- Adding a new field variant to either side requires touching exactly: the field enum + one `apply_to` arm (+ serde as needed) — demonstrate in the PR by the diff shape of a trial variant or by doc.
- No hand-written `*Type` discriminant enums remain (grep).
- Model delta apply no longer clones payloads on Noop/Delete.
- `./test.sh` green; benches quoted.

## Pointers

`lib/baumhard/src/gfx_structs/{area_fields.rs,area_mutators.rs,area.rs,element.rs,mutator.rs}`; `lib/baumhard/src/gfx_structs/model/{mutator.rs,glyph_model.rs,matrix.rs,component.rs}`; CONVENTIONS §B4, §B7; CODE_CONVENTIONS §2 ("unify the shapes"), §5.
