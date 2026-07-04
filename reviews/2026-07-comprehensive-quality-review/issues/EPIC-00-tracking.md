# EPIC-00: Quality-review tracking epic — July 2026 comprehensive review

**This is the dashboard issue.** It indexes all 53 review issues, carries the dependency graph, and tracks completion. File this issue LAST, after all others, substituting real `#` numbers into the checklists below (the filing manifest is `reviews/2026-07-comprehensive-quality-review/MANIFEST.json`; the graph is `DEPENDENCIES.md` in the same directory).

## Background

A full-depth review of baumhard, mandala, maptool, and mandala_derive was performed in July 2026 against the project's own conventions: ten parallel area reviews plus a cross-cutting SSOT sweep, with all high-severity findings independently re-verified (several empirically). Baseline: 2669 tests green at commit `59cd115`. The synthesis, per-area evidence reports, and verification log live under `reviews/2026-07-comprehensive-quality-review/` (branch `claude/repo-quality-review-qm6p15` until merged).

Issue sizing: **task** = small sharp fix; **feature** = one coherent PR; **epic** = staged, multi-PR checklist (P1-22, P1-23, P1-24, P1-26, P1-27, P1-29, P1-30, P1-33, P1-35, P2-36, P2-41, P3-53).

## Wave 1 — Safety (P0s + walker/mutation correctness; parallel-friendly)

- [ ] P0-01 DeleteNode undo corruption (bug)
- [ ] P0-02 custom-mutation sync-back gap / dead Toggle (bug)
- [ ] P0-03 animation completion double-apply (bug; after P0-02 preferred)
- [ ] P0-04 delete_front_unicode(s, 0) eats a grapheme (bug)
- [ ] P0-05 parent_id cycle hang on load (bug)
- [ ] P0-06 metric_cache FONT_SYSTEM self-deadlock (bug)
- [ ] P0-07 swapchain-format two sources of truth (bug)
- [ ] P1-08 RepeatWhile channel alignment (bug)
- [ ] P1-09 Predicate comparator semantics (bug)
- [ ] P1-10 mutation-surface completeness (bug)
- [ ] P1-11 subtree-AABB staleness (bug)
- [ ] P1-20 `color …=accent section=K` var double-wrap (bug)

## Wave 2 — Enablers

- [ ] P1-12 border line_height_pt threading — **blocks P1-22**
- [ ] P1-23 O(N²) model walks: child index + fold memo (epic) — soft-blocks P2-36/P2-37
- [ ] P1-25 validation SSOT into baumhard — soft-blocks P1-14
- [ ] P1-17 IME insertion primitive — soft-blocks P1-30
- [ ] P2-43 baumhard seam hygiene (EventSubscriber shape, Tree phantom fields, §B6 decision) — blocks P2-40's crossbeam item

## Wave 3 — Consolidations (each ends with less code)

- [ ] P1-22 dual-pipeline consolidation (epic; **blocked by P1-12**)
- [ ] P1-24 area/model mutation twins (epic)
- [ ] P1-26 document setter fan-out (epic) — soft-blocks P1-15
- [ ] P1-28 console resolver + dual dispatch (before/with P1-27)
- [ ] P1-27 console verb framework (epic)
- [ ] P1-29 native/WASM shared cores (epic)
- [ ] P1-30 single-line editors + ThrottledInteraction lifecycle (epic)
- [ ] P2-40 workspace hygiene (partially blocked by P2-38, P2-43)
- [ ] P2-41 dead-code sweep + remove crate-wide allow (epic; after P1-22 step 1)
- [ ] P2-49 config-loader scaffolding

## Wave 4 — Correctness backlog, perf, hardening

- [ ] P1-13 convert --legacy portals + atomic writers (bug)
- [ ] P1-14 verify↔spec drift (bug; uses P1-25's constants)
- [ ] P1-15 palette system: wire or re-document (**decision gate**)
- [ ] P1-16 CRLF/trailing line model (bug)
- [ ] P1-18 build.rs determinism + rerun-if-changed + UTF-16 names (bug)
- [ ] P1-19 flat_mutations guard + covers_reach anchoring (bug)
- [ ] P1-21 broken dormant primitives: fix or delete (bug)
- [ ] P1-31 dispatch-funnel gaps: picker/LeftDrag/EditSelection (bug)
- [ ] P1-32 keybind config SSOT + cancel_mode template fix
- [ ] P1-33 renderer re-shape granularity (epic)
- [ ] P1-34 renderer hitbox maps → BVH
- [ ] P1-35 touch vocabulary: tap/pan/pinch (epic; with P1-29)
- [ ] P2-36 projection hot-path perf bundle (epic; **blocked by P0-06**)
- [ ] P2-37 interaction rebuild tiers (after P1-23/P1-30 preferred)
- [ ] P2-38 grapheme_chad API gaps — blocks P2-40's unicode-segmentation item
- [ ] P2-39 loader double-parse + unknown-key warnings
- [ ] P2-42 unwrap posture (26-site inventory)
- [ ] P2-44 bench discipline (with P3-53's bench-ID renames)
- [ ] P2-45 release logging policy (**decision gate**)
- [ ] P2-46 constants SSOT (after P2-41 preferred)
- [ ] P2-47 model serde + SectionRange meaning (**decision gate on part C**)
- [ ] P2-48 test gaps + small-fix roundup
- [ ] P2-49 config-loader scaffolding (listed in wave 3; check there)

## Wave 5 — Documentation (after the code settles)

- [ ] P3-50 CONCEPTS.md accuracy sweep
- [ ] P3-51 format/ + convention-doc reconciliation
- [ ] P3-52 in-code doc-corruption sweep
- [ ] P3-53 American English sweep (epic; last — touches everything)

## Hard blockers (summary)

- P1-12 → P1-22 (the dead flat path holds the only correct `line_height_pt` handling; port before deleting)
- P0-06 → P2-36 (metric-cache key redesign rides the lock redesign)
- P2-38 → P2-40 (migrate call sites before removing `unicode-segmentation`)
- P2-43 → P2-40 (remove the dead Sender param before dropping `crossbeam-channel`)

## Human-decision gates

P2-45 (release logging A/B) · P2-47-C (SectionRange meaning) · P1-15 (wire palettes vs re-document) · P2-43-C (§B6: wire region-index maintenance vs rewrite the doc). Each is a small diff once decided; agents should not guess these.

## Definition of done for the epic

Every checkbox closed; `./test.sh` green throughout; CODE_CONVENTIONS §3's pipeline description and TEST_CONVENTIONS §T10 updated to the converged reality; a final grep-audit confirms the sweep classes stay at zero (TODO markers, bare unwraps, British spellings, dead flat-pipeline symbols).
