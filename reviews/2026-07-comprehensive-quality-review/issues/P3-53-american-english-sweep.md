# P3-53: American English sweep — ~600 British spellings across code comments, identifiers, user-facing strings, bench IDs, and docs

**Severity:** P3 (explicitly mandated: CLAUDE.md §6 "Use American English for consistency"; CLAUDE.md §2 forbids skipping cosmetic-only fixes) · **Area:** repo-wide

## Inventory (verified counts)

- **Code comments (.rs)**: ~577 occurrences — colour 233, centre 110, behaviour 96, recognise* 60, honour* 30, serialise* 28, initialise* 15, quantis* 12, normalise* 10, grey 7, catalogue 3, licence 1, synchronis* 1. Heaviest: baumhard core/primitives.rs (17), touch_gesture.rs ("recogniser" ×20 in docs against the American `recognizer` identifier in the same file).
- **Production identifiers**: `fn apply_section_colours(` (`console/commands/color.rs:330`); `let recognised` (`run_native.rs:329`, `run_wasm/event_touch.rs:57`).
- **User-facing strings**: "not recognised" (section verb error + action_core warn + a test pinning the spelling), "internal: unrecognised corner" (×3), "cancelled" (border preview), "colours"/"colour axis" (completion hint + error), pattern-parse error "unrecognised escape" (border_pattern.rs:136 — surfaced verbatim in the console), "falling back to single colour" (border.rs:1395), stress-tool "serialise mindmap" error.
- **Bench IDs (two-file §B8 renames)**: `"region_indexer_initialise"` (test_bench.rs:264 — also cited by name in CONVENTIONS §B6; the criterion baseline name differs from the American `do_region_indexer_initialize` fn it calls) and `"shape_ellipse_contains_centre_and_rim"` (:203, coupled to `do_shape_ellipse_contains_centre_and_rim` in shape_tests.rs).
- **Test identifiers**: `test_new_initialises_*` (×7 throttled_interaction files), `..._centre...` (camera/hit tests), `parse_unrecognised_...` (border_pattern), `..._serialises_...` (stress map), `test_explicit_channel_zero_honoured_...`.
- **Docs**: CONCEPTS.md 47, SECTIONS_BORDERS_RESIZE_PLAN.md 37, format/sections.md 13, others 1-3 each; also run.sh ("artefact", "optimisation"), build.sh ("unoptimised"), rustfmt.toml ("behaviour"), mandala_derive ("defence").

## Fix plan

1. Mechanical word-boundary sweep of comments/docs/prose (colour→color, centre→center, behaviour→behavior, -ise→-ize family, honour→honor, grey→gray, licence→license, artefact→artifact, neighbourhood→neighborhood).
2. Identifier renames (compiler-checked): `apply_section_colours`, the two `recognised` locals, the `test_*initialises*`/`*centre*`/`*unrecognised*`/`*serialises*`/`*honoured*` test fns.
3. **Careful set — same-commit pairings**: user-facing strings that tests pin (update test + string together, e.g. the "not recognised" pin at section/mod.rs:1639); the two bench IDs (rename bench label + `do_*` fn + CONVENTIONS §B6 reference together per §B8's two-file rule; accept one criterion baseline reset and say so in the commit message).
4. Do the sweep in 2-3 commits (baumhard / mandala / docs+scripts) to keep review tractable. Word-boundary regex only; review every hit in a string literal by hand.

## Acceptance criteria

- `rg -i "colour|behaviour|recognis|initialis|serialis|centre|honour|artefact|unoptimis|quantis|normalise|synchronis|catalogue|licence\b" --type rust` returns zero hits outside deliberate exceptions (none expected).
- `./test.sh` green (pinned-string tests updated in the same commits); `./test.sh --bench` runs (bench rename verified).

## Pointers

CLAUDE.md §6, §2; CONVENTIONS §B8 (bench two-file rule); crosscutting findings sweep 12 for the full per-file tally.
