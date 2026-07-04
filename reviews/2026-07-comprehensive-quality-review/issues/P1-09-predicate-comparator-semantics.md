# P1-09: Predicate comparator semantics are internally inconsistent; one arm drops the negation flag entirely

**Severity:** P1 (mutation-language correctness; serde-authorable) · **Area:** baumhard/gfx_structs/predicate

## Context

`Predicate` + `Comparator` form the condition language of `RepeatWhile` custom mutations — pure serde data authored in JSON (the primary extensibility seam, CONCEPTS §4). `compare_f32` documents the convention: "`a` — the element-side value (left operand), `b` — the reference value" (`predicate.rs:107-122`).

## Problem

`lib/baumhard/src/gfx_structs/predicate.rs` — the hand-written arms disagree with the convention and with each other:

- **Id follows it**: `(element.unique_id() > *id) != *negation` (:224). ✔
- **Channel inverts it**: `(*channel > element.channel()) != *negation` (:207) — "GreaterThan" here means *reference greater than element*.
- **Model Layer inverts too**: `(*layer > target_model.layer) != *negation` (:441); **GlyphLines** compares `lines.len() > matrix.len()` reference-left (:440).
- **Layer under LessThan drops the negation flag**: `Layer(layer) => *layer < target_model.layer` (:464) — every sibling arm ends `!= *negation`; here `LessThan(true)` (documented as `>=`) evaluates as plain `<`. Outright bug.
- **Exists on Channel** falls into `_ => false` (:209) even for `Exists(false)` (documented "returns true unconditionally"), while Id handles it (:226).

`predicate_tests.rs` pins only `compare_f32` and equality/flag paths; none of these directional arms is tested. A DSL author cannot predict which side their reference lands on — `GreaterThan` on `Scale` means "element > ref", on `Channel` it means "ref > element".

## Fix plan

1. Normalize **every** arm to the documented element-side-left convention (behavior change is sanctioned pre-V1 per CODE_CONVENTIONS §10; there are no known user-authored predicates relying on the inverted arms).
2. Restore `!= *negation` on the Layer/LessThan arm.
3. Give Channel an `Exists` arm consistent with Id.
4. Add a truth-table test in the `OVERLAPS_TEST` lazy-static style (§T3): field × comparator × negation, covering Channel, Id, Layer, GlyphLines, Scale, and flag fields — both directions of each inequality.

## Acceptance criteria

- One documented convention holds for every field arm (spot-check via the truth table).
- New tests fail on current main for the inverted/dropped arms.
- `./test.sh` green; update `format/mutations.md` if it documents comparator semantics.

## Pointers

`lib/baumhard/src/gfx_structs/predicate.rs:107-122, 205-227, 435-471`; `lib/baumhard/src/gfx_structs/tests/predicate_tests.rs`; CONCEPTS §2 (Predicate/Comparator); format/mutations.md.
