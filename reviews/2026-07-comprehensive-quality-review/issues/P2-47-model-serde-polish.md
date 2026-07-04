# P2-47: Model serde polish — TextRun's all-required fields break the format doc's own example; `size_pt: u32` vs f32 everywhere else; `label: null` emitted per unlabeled edge; SectionRange's field means two different things

**Severity:** P2 (author-facing format friction + one genuine type incoherence) · **Area:** baumhard model + mandala selection

## Problem A — TextRun serde strictness contradicts the docs

`model/node.rs:401-423`: `TextRun` has no serde defaults (only `hyperlink` is Option). `format/text-runs.md:12-16`'s own example omits `italic`/`underline` and **fails the typed loader** ("missing field"). CONCEPTS calls font/size_pt/color "optional" — all three are required. And `size_pt: u32` while every sibling size field (`GlyphBorderConfig.font_size_pt`, `GlyphConnectionConfig.font_size_pt`) is f32 — `"size_pt": 14.5` is a parse error; schema.md types it "number".

**Fix:** `#[serde(default)]` the three bools + `font`/`color` (empty = unpinned, matching fonts.md), defaulted `size_pt`; consider f32 for size_pt (update fixtures same commit per §10). Or keep required and fix the three docs — but defaults match the documented intent.

## Problem B — serde hygiene nits

- `MindEdge.label` is the only Option field without `skip_serializing_if` — every unlabeled edge serializes `"label": null` (edge.rs:44). Fix + re-save fixtures same commit.
- `serialized.rs:87-105` claims "omits empty-default fields" but always writes `behavior`.
- `border.rs:350,991,1374` hardcode `14.0` instead of `default_border_font_size()` (three copies of a default that has a named fn).
- `Canvas` has no `Default` — hand-constructed field-by-field at 6+ sites (model, test_helpers, tests, maptool helpers).

## Problem C — `SelectionState::SectionRange.range` has two contradictory meanings

`document/types.rs:193-200` documents the field as a pair of **section indices**; `types.rs:393-402` (`selected_range`) and CONCEPTS §5 document it as a **grapheme sub-range within one section**. Consumers split accordingly: section-index consumers (`nodes/border.rs:978-990`, `section_structure.rs:149-168`) vs grapheme consumers (`console/commands/font.rs:338-346`, `color.rs:176-186`, `traits/view.rs:626-682`, `cross_dispatch/style.rs:136`). The sole intended producer (text-editor shift-select lift, `text_edit/editor.rs:257-294`) currently emits `Section` and discards the range — and its comment asserts "every SectionRange consumer … interprets the range field as section indices", which is false. The variant is unconstructable in production today, so the grapheme verb arms and the documented picker/Cut/Paste contracts are dead code documented as live; the next contributor wiring shift-select per CONCEPTS reintroduces the collision silently (grapheme (3,17) misread as "sections 3..=17").

**Fix:** pick one meaning. If grapheme (matches CONCEPTS + the picker plumbing): rename the field `grapheme_range`, fix `live_selection_section_pairs`/structural-cleanup to treat the variant as single-section, wire the editor lift to emit it. If section-span: rename `section_span`, delete the grapheme arms + `TargetId::Section.range`, fix CONCEPTS. Either way add a newtype so the units can't cross again, and a test constructing the variant end-to-end.

## Acceptance criteria

- text-runs.md's example loads as written.
- Fresh-loaded canonical fixtures re-save byte-identically (determinism tests keep passing) after the fixture refresh.
- Exactly one meaning of `SectionRange.range`, produced and consumed end-to-end, pinned by test.
- `./test.sh` green.

## Pointers

`lib/baumhard/src/mindmap/model/{node.rs,edge.rs,canvas.rs}`; `lib/baumhard/src/mindmap/custom_mutation/serialized.rs`; `src/application/document/types.rs`; `src/application/app/text_edit/editor.rs:257-294`; format/text-runs.md, schema.md; CONCEPTS §5 (SelectionState); CODE_CONVENTIONS §10.
