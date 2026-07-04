# P2-41: Remove the crate-wide `#![allow(dead_code)]` and sweep the verified dead-code inventory

**Severity:** P2 (§5's no-dead-code covenant is currently unenforceable) · **Area:** both crates · **Found independently by three reviewers**

## Problem

`src/main.rs:3` carries a bare crate-wide `#![allow(dead_code)]` over the ~83K-LOC app crate — the compiler backstop for CODE_CONVENTIONS §5 ("no dead code") is off. Verified consequences already found behind it: a regression test whose `#[test]` got swallowed into a doc comment and never runs (`console/commands/section/mod.rs:1353-1355` — verified: "running 0 tests"); `read_edge_label` with zero call sites; a duplicate `#[test]` attribute compiling with a suppressed lint. Baumhard demonstrates the right idiom: per-item allows with seam comments.

## Verified dead inventory (delete per §10, or convert to real seams with named consumers)

**mandala:**
- `RedrawMode` machinery: `OnRequest`/`FpsLimit` arms unreachable (mode set once to `NoLimit`); the dead `FpsLimit` arm contains a `Duration` **underflow panic** primed to fire the day someone wires it (`renderer/mod.rs:711-718, 841-873`); `timer`/`target_duration_between_renders`/`last_render_time` serve only the dead arm.
- Retired picker surfaces: `ColorPickerOverlayGeometry.{target_label, preview_hex, selection_hint}` produced per rebuild (two String allocs) and read by nothing; phantom 32-font chip-width clamp still capping mobile font size (`color_picker/compute_sizing.rs:87-88`); "chip" vocabulary across ~10 comment sites.
- Never-emitted decrees `ReinitAdapter`, `StopRender`; never-written `console_overlay_buffers` field still chained into the draw path; uncalled `fit_camera_to_scene`; production-unused `degrees_to_hue_slot`; false `#[allow(unused_imports)]` justification on renderer re-exports.
- `build_mutation_registry_with_user` (no callers); stale `#[allow(unused_imports)]` + "until commit 5 lands" comment on a now-used re-export (`document/mod.rs:78-84`); completion identity re-map ceremony (`console_input/completion.rs:32-41`).
- Dead flat-pipeline consumers (tracked in P1-22 — coordinate, don't double-delete).

**baumhard:**
- `core/animation.rs`: entire module unused with an API shape that cannot work as documented (`AnimationDef<T: Mutable>` boxes values not mutators; `update()` takes no `&mut`, returns nothing; `Mutable` has zero implementers) — rewrite when the timing `Followup` work lands, or delete and point CONCEPTS at `custom_mutation/timing.rs`.
- `util/palettes.rs`: 108 lines, 13 lazy statics, zero references.
- Anchor system (`core/primitives.rs:580-777`): `Anchor`/`AnchorBox`/`AnchorPoint`/`AnchorTarget`/`Flag::Anchored` — zero references, no layout solver exists; `Positioned`/`Bounded` traits with zero implementers. Splitting primitives.rs (regions/apply/flags) is the natural companion refactor (§6).
- `hex!`/`rgb!` macros (dead + `hex!` panics on odd-length input, contradicting its own doc); `get_font_source`/`get_some_font` (the latter drags `rand` into production and carries a bare unwrap); `glyph_ink_height` + `shape_ink_height` + `INK_HEIGHT_CACHE` (full shaping work to return a constant; zero callers; module doc advertises behavior it doesn't have); `GlyphComponentField` (exported, referenced nowhere); `DeltaGlyphArea`/`GlyphAreaField` `Add` impls (no production callers + spurious mismatch warn on the Operation arm); `Tree::import` (zero callers AND violates the invalidation discipline it documents); empty `mod test {}` in grapheme_chad.

## Fix plan

1. Land the inventory deletions above (each trivially greppable; coordinate the P1-22 overlap).
2. Remove `#![allow(dead_code)]` from src/main.rs; triage remaining warnings — delete, or annotate item-level with a comment naming the future consumer (§7 seams).
3. Add the two lint-hygiene fixes: the swallowed `#[test]` (put the attribute on its own line or delete the duplicate test), the doubled `#[test]` attribute (`section_structure.rs:714`).

## Acceptance criteria

- `cargo check` warning-clean without the crate-level allow (remaining allows are item-level with justifications).
- Grep-clean for every named dead item.
- `./test.sh` green; test count does not drop except for deliberately deleted dead tests (list them in the PR).

## Pointers

CODE_CONVENTIONS §5, §7 (seams need reachable surfaces), §10 (delete rather than deprecate); source findings: renderer/core/fontutil/console/crosscutting reports in `reviews/2026-07-comprehensive-quality-review/`.
