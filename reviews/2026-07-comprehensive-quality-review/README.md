# Comprehensive Repository Quality Review — July 2026

**Scope:** the full workspace — `baumhard` (lib/baumhard, ~50K LOC, mostly human-written), `mandala` (src/, ~83K LOC, mostly AI-written), `maptool` (crates/maptool, ~4K LOC), `mandala_derive`, plus all convention documents, format specs, scripts, manifests, and CI.

**Method:** ten parallel deep-review passes, each reading its slice end-to-end against the project's own conventions (CODE_CONVENTIONS.md, lib/baumhard/CONVENTIONS.md, TEST_CONVENTIONS.md, CONCEPTS.md, format/) — not generic Rust style. A dedicated cross-cutting pass swept the seams between modules and crates (constants, color, geometry, grapheme discipline, cosmic-text containment, parallel enums, serde shapes, platform pairs, logging, unwrap posture, spelling). Every high-severity claim was then independently re-verified against the source; several were confirmed empirically (probes run against the built crate, reproduced CLI failures, `cargo` invocations). Baseline: `./test.sh` = **2669 tests, 0 failures** on the reviewed commit.

**Deliverables:**
- `issues/` — **53 self-contained issue drafts** (P0-01 … P3-53), each with evidence, file:line references, a concrete fix plan, and acceptance criteria, written so an AI agent can pick any one up without further context. GitHub Issues is currently **disabled** on this repository (the API returns 410), so they live here; enable Issues in Settings → General → Features and they can be bulk-filed verbatim.
- `findings/` — the ten full review reports plus the verification log, preserving all evidence, per-file line references, and (importantly) the explicit **"checked and found CLEAN"** lists that bound what was audited.
- This document — the synthesis: what the codebase is, where it is strong, and where the leverage is.

---

## Executive summary

This is a genuinely unusual codebase: 137K lines with **zero TODO/FIXME/HACK markers, zero `unsafe`, zero custom error types, no stray threads, and a green 2669-test suite** — the written conventions are not aspirational decoration; most of them are demonstrably enforced habit. The architecture documents (CONCEPTS.md, the conventions) are among the best-maintained I have reviewed, and many subsystems match them: the mutation-first tree substrate, the BVH hit-testing, the adaptive drag throttle, the macro privilege gate, the shape/zoom/camera primitives, and the grapheme discipline in the live editors are exemplary work.

The debt is equally characteristic, and it clusters into four families:

1. **Bridges between layers are where correctness dies.** The five P0-class defects all live at hand-maintained sync points: model⇄tree sync-back (grow-font is a silent no-op; Toggle behavior is dead), delete⇄undo ID minting (real data corruption), animation⇄commit (double-apply), loader⇄walkers (cycle hang), cache⇄lock discipline (latent self-deadlock). The pattern: two subsystems each correct in isolation, coupled by an implicit contract nothing enforces.

2. **Sync-by-comment instead of sync-by-construction.** The single most common finding shape, in both crates: two (or four) copies of knowledge held together by a "Mirrors X — keep in sync" comment. Validation rules mirrored byte-for-byte across crates, the walker's two alignment paths with different correctness guarantees, area/model mutation twins, verb grammars encoded 2–4× per console verb, native/WASM behavior bodies, the keybind field/resolve tables, selection cyan in four byte-disagreeing forms, a 4px-vs-5px drag threshold under a "same value" comment. In nearly every case reviewed, at least one copy had **already drifted** — this is not a hypothetical risk class.

3. **Phantom machinery documented as live.** A ring of subsystems exists in the docs but not in the wires: the palette color cascade (documented as *the* theming system; `resolve_theme_colors` has no production caller), the region-index maintenance pipeline (§B6 commands an invariant no code implements), `core/animation.rs` (unusable API shape, zero users), the flat RenderScene consumers (dead, yet every frame still pays for their inputs twice), Tree's seam fields (private, unreachable, described as "used narrowly today"), the picker chip row (retired, still steering mobile font sizing). For a foundation crate whose conventions say "a doc comment that lies is worse than no doc comment", this is the most dangerous drift family — and the crate-wide `#![allow(dead_code)]` on the app removed the compiler's ability to notice.

4. **The mobile budget is leaking at the seams.** The per-frame path does substantial dead or duplicated work: dead scene elements manufactured every frame, border/portal styles resolved twice per frame, O(N²) model walks per rebuild and per drag frame, ~520 cosmic-text shapings per picker hover, full-arena re-shapes per resize drain, String keys and interning churn in per-frame maps. None of it shows on a desktop; all of it burns the phone battery the conventions name as the binding budget.

None of this contradicts the overall verdict: the foundations are sound, the invariants that matter most (Unicode, undo coverage breadth, single-threaded discipline, panic posture) are mostly held, and the fix surface is well-defined. The 53 issues are ordered so that the seven P0s and the first half of the P1s are small, sharply-scoped corrections; the architectural items (P1-22 … P1-35) are staged consolidations that each end in *less* code than they started with.

---

## The P0s — verified defects to fix first

| # | Defect | One-line trigger |
|---|--------|------------------|
| [P0-01](issues/P0-01-deletenode-undo-corruption.md) | DeleteNode undo **destroys node data** when the deleted node was the highest-numbered root with children (remove-before-mint ID collision + silent HashMap overwrite on undo) | Delete the newest root with children, Ctrl-Z |
| [P0-02](issues/P0-02-custom-mutation-syncback-gap.md) | `sync_node_from_tree` never reads tree `scale` back → **grow/shrink-font-2pt are silent no-ops**; Toggle behavior dies at the post-dispatch rebuild; junk undo entries accumulate | `mutation apply grow-font-2pt` |
| [P0-03](issues/P0-03-animation-completion-double-apply.md) | Animation completion applies the full relative mutation **on top of** the lerped state (~double delta) and snapshots undo mid-lerp | Any timed relative mutation ≥2 frames |
| [P0-04](issues/P0-04-delete-front-unicode-zero.md) | `delete_front_unicode(s, 0)` **eats one grapheme** (empirically confirmed); boundary-aligned `GlyphLine` range-deletes hit n=0 naturally | Component range-delete at a boundary |
| [P0-05](issues/P0-05-parent-id-cycles-hang.md) | Loader accepts `parent_id` cycles → `is_hidden_by_fold` **hangs** / `all_descendants` **stack-overflows** on first touch (found independently twice) | `open` a hand-edited/hostile map |
| [P0-06](issues/P0-06-metric-cache-font-system-deadlock.md) | `metric_cache` raw-blocking-acquires FONT_SYSTEM; a cold key measured while the renderer holds the guard is a **permanent same-thread deadlock** (ordering accident currently masks it) | Cold border-glyph metric under the render guard |
| [P0-07](issues/P0-07-swapchain-format-two-sources.md) | Atlas + rect pipeline hardcode `Bgra8UnormSrgb`, surface uses `capabilities.formats[0]` — latent fatal format mismatch exactly on the WebGL/mobile target (the code comment contradicts itself) | WASM on a backend reporting Rgba8 first |

## Where the codebase is genuinely strong

A fair review states what was checked and found clean — the full lists are in each `findings/*.md`; highlights:

- **Baumhard's newer primitives are exemplary**: `shape.rs`, `zoom_visibility.rs`, `camera.rs` (correct math, cost-documented, exhaustively tested); `align_child_walks`' sorted channel-merge; MapChildren's zip semantics incl. nested-instruction forwarding; the BVH descent (§B7 zero-alloc, correct tie-breaks); the five live `ColorFontRegions` mutation primitives (four of five verified correct — the fifth is dormant and P1-21); RegionParams/RegionIndexer math (exhaustive brute-force tests — the issue is that nothing uses it, not that it's wrong).
- **The dispatch architecture is real**: the Action funnel holds for nearly every Compatible action; ~15 of ~18 parametric arms genuinely share one `pub(crate)` core with the console verbs; the macro privilege gate is single-sourced, fail-closed, compiler-backed via `mandala_derive::ActionClassify` (itself a model SSOT mechanism), and heavily tested including the smuggling pattern.
- **Grapheme discipline in live paths**: all four editors + console cursor math route through `grapheme_chad`; emoji/ZWJ/combining fixtures throughout; the cosmic-text bridges slice grapheme-correct (verified against ZWJ-family and flag sequences).
- **cosmic-text containment is perfect at the code level**: zero imports outside `lib/baumhard/src/font/` and the sanctioned renderer — every other hit is a doc comment. (The dead direct dependency in mandala's manifest is the one hole — remove it and containment becomes compiler-enforced.)
- **Lock discipline in the renderer**: frame paths `try_write` + degrade; rebuild paths use the timeout helper; the 9× halo stamping runs under a single acquisition; no lock held across GPU submit. (metric_cache is the exception — P0-06.)
- **Test culture**: 2669 green tests; all 13 UndoAction variants round-trip tested; loader migration rejections carry §9-quality messages with `maptool convert` pointers, all tested; maptool's `apply` is atomic, deadlock-safe subprocess piping with regression-pinned corruption cases; HashMap-order-determinism hardening is pervasive.
- **Zero TODO/FIXME/HACK, zero `unsafe`, zero commented-out code** across all three crates — §5's letter, held at 137K lines.

## Thematic analysis

### Duplication and single-source-of-truth (the largest family — ~20 of 53 issues)

The highest-leverage unifications, in rough order of blast radius: the **console verb framework** (P1-27: one grammar per verb instead of 2–4 hand-synced encodings across 20 verbs), the **document setter envelopes** (P1-26: the undo-snapshot template exists as three helpers yet is open-coded at ~12 more sites; the grow-pass tail at 12+), the **area/model mutation twins** (P1-24: seven hand-written discriminant enums that strum derives for free; delta plumbing duplicated; drift already shipped), **native/WASM cores** (P1-29), **validation into baumhard** (P1-25: byte-identical rules and message strings hand-mirrored across two crates, celebrated as such by the docs), and the **single-line editor pair** (P1-30: ~230-line structural mirrors). The repo repeatedly demonstrates it knows the right pattern — `mutate_edge`, `effective_size`, zoom-cascade helpers, the border preset table, `ActionClassify` — the work is applying the pattern it already invented to the sites that predate it.

### Architecture

The **dual-pipeline consolidation** (P1-22) is further along than CODE_CONVENTIONS §3 admits: every visual role already reaches the GPU through the Baumhard tree; the flat consumers are dead code. What remains is deletion plus moving three "courier" roles home — after which `RenderScene` collapses into connection sampling + cache. The staged plan in the issue is four independently-shippable steps, each reducing per-frame work. Two real cross-pipeline casualties need landing first/alongside: the `line_height_pt` fix (P1-12 — the "gappy rails" behavior only exists on the dead path) and the double style resolution (inside P1-22 step 2).

The **walker** has one seam with two implementations of child alignment at different correctness levels (P1-08) — the RepeatWhile path needs the same sorted-merge hardening `align_child_walks` already got, plus a corrected advance rule.

The **event/seam layer of baumhard** (P2-43) needs its shapes corrected while there are still zero consumers: `Rc<RefCell>` instead of `Send+Sync` Mutex closures, attachable seams instead of private dead fields, no forced crossbeam channel in a "no channels" architecture.

### Performance (mobile budget, §B1)

Beyond the P1-22 dead work: O(N²) `children_of`/`all_descendants`/fold walks on rebuild and drag paths (P1-23 — the single biggest scaling fix); re-shape granularity in the renderer (P1-33: ~520 shapings per picker hover, full-arena re-shape per resize drain, String-keyed buffer maps); the projection hot-path bundle (P2-36: O(samples×nodes) clip filter, per-call arc-length allocations, metric-cache String keys on every hit, identity-only mutator signatures); and interaction tier misuse (P2-37: every native click pays a full rebuild while the WASM path already right-sizes; rect-select rebuilds the arena per drain under a comment calling it "lightweight").

### Conventions compliance scorecard

| Convention | Verdict |
|---|---|
| §5 no TODO/FIXME/HACK, no unsafe, no commented-out code | **Held, fully** (remarkable at this size) |
| §3 single-threaded, no channels | Held — one dead crossbeam Sender param (P2-43) |
| §3 dispatch funnel | Held for ~90% — picker modal, LeftDrag, WASM wheel/double-click outside (P1-31, P1-29) |
| §3 undo coverage | Broad and tested — three deep defects (P0-01/02/03) + no-op-push hygiene |
| §B3 grapheme discipline | Held in live paths — two n=0/CRLF primitive bugs (P0-04, P1-16), per-char IME loops on 2 surfaces (P1-17), hand-rolled app-side scans (P2-38) |
| §B5 cosmic-text containment | **Held perfectly in code**; metric_cache lock bypass is the §B5 hole (P0-06) |
| §B7 hot-loop allocations | Leaking at named sites (P1-33, P2-36) |
| §B9 doc coverage | ~92–97% with genuine cost notes — gaps listed (P3-52 + per-issue) |
| §B8 bench-reuse | **Drifted badly** — ~156/299 bodies unbenched, 7 modules never imported, 1 wrapper calls the wrong body (P2-44) |
| §9 no bare unwrap | 26 in production (complete inventory, P2-42); release builds compile out the mandated log channel (P2-45) |
| §5 no dead code | **Unenforceable** under crate-wide `#![allow(dead_code)]`; verified inventory in P2-41 |
| CLAUDE.md §6 American English | ~600 violations incl. identifiers, user-facing strings, bench IDs (P3-53) |
| Docs = truth (§8/§B9) | The weak axis: CONCEPTS/format/CONVENTIONS each contradict shipped code in load-bearing places (P3-50/51/52; §B6's nonexistent index pipeline; the §3 macro-gate paragraph saying gates are dormant when all four tiers load) |

### Process observation (for the human maintainer)

Two systemic notes worth acting on beyond individual issues. First, the `#![allow(dead_code)]` + no-lint-gate combination is what allowed a swallowed `#[test]`, a doubled `#[test]` attribute, dead modules, and phantom seams to accumulate invisibly in an agent-written crate — removing the allow (P2-41) and treating `./test.sh --lint` less advisorily would make several finding classes structurally impossible. Second, the recurring "merge-damage" doc pattern (fused doc blocks, glued `///`, redaction-artifact sentences — P3-52) suggests review passes that trimmed code without re-reading adjacent docs; the §-renumbering drift (§4/§6/§7 → §9/§7) suggests convention edits without a follow-up citation sweep. Both are cheap to sweep now and cheap to keep clean once zeroed.

---

## Issue index

**P0 — fix first:** [01 DeleteNode undo corruption](issues/P0-01-deletenode-undo-corruption.md) · [02 sync-back gap / dead Toggle](issues/P0-02-custom-mutation-syncback-gap.md) · [03 animation double-apply](issues/P0-03-animation-completion-double-apply.md) · [04 delete_front_unicode(0)](issues/P0-04-delete-front-unicode-zero.md) · [05 parent_id cycle hang](issues/P0-05-parent-id-cycles-hang.md) · [06 metric_cache deadlock](issues/P0-06-metric-cache-font-system-deadlock.md) · [07 swapchain format](issues/P0-07-swapchain-format-two-sources.md)

**P1 correctness:** [08 RepeatWhile alignment](issues/P1-08-repeatwhile-channel-alignment.md) · [09 Predicate comparators](issues/P1-09-predicate-comparator-semantics.md) · [10 mutation-surface completeness](issues/P1-10-mutation-surface-completeness.md) · [11 subtree-AABB staleness](issues/P1-11-subtree-aabb-staleness.md) · [12 border line_height dropped](issues/P1-12-border-line-height-dropped.md) · [13 convert --legacy portals](issues/P1-13-convert-legacy-portals.md) · [14 verify↔spec drift](issues/P1-14-verify-spec-drift.md) · [15 palette system unwired](issues/P1-15-palette-system-unwired.md) · [16 CRLF line model](issues/P1-16-grapheme-line-model.md) · [17 IME cursor drift](issues/P1-17-ime-insertion-cursor-drift.md) · [18 build.rs determinism](issues/P1-18-buildrs-determinism.md) · [19 flat_mutations + scope gate](issues/P1-19-flat-mutations-and-scope-gate.md) · [20 var double-wrap + hex validators](issues/P1-20-color-value-bugs.md) · [21 broken dormant primitives](issues/P1-21-broken-dormant-primitives.md)

**P1 architecture:** [22 dual-pipeline consolidation](issues/P1-22-dual-pipeline-consolidation.md) · [23 O(N²) model walks](issues/P1-23-model-walk-complexity.md) · [24 area/model twins](issues/P1-24-area-model-mutation-twins.md) · [25 validation SSOT](issues/P1-25-validation-ssot-baumhard.md) · [26 setter fan-out](issues/P1-26-document-setter-fanout.md) · [27 console verb framework](issues/P1-27-console-verb-framework.md) · [28 console resolver/dual dispatch](issues/P1-28-console-resolver-and-dual-dispatch.md) · [29 native/WASM cores](issues/P1-29-native-wasm-shared-cores.md) · [30 editors + throttle trait](issues/P1-30-editor-and-throttle-unification.md) · [31 funnel gaps](issues/P1-31-dispatch-funnel-gaps.md) · [32 keybind SSOT](issues/P1-32-keybind-config-ssot.md) · [33 re-shape granularity](issues/P1-33-renderer-reshape-granularity.md) · [34 hitbox maps → BVH](issues/P1-34-renderer-hitbox-maps.md) · [35 touch vocabulary](issues/P1-35-touch-vocabulary.md)

**P2:** [36 projection hot-path perf](issues/P2-36-projection-hotpath-perf.md) · [37 rebuild tiers](issues/P2-37-interaction-rebuild-tiers.md) · [38 grapheme_chad gaps](issues/P2-38-grapheme-chad-api-gaps.md) · [39 loader double-parse/unknown keys](issues/P2-39-loader-costs-and-unknown-keys.md) · [40 workspace hygiene](issues/P2-40-workspace-hygiene.md) · [41 dead-code sweep](issues/P2-41-dead-code-sweep.md) · [42 unwrap posture](issues/P2-42-unwrap-posture.md) · [43 seam hygiene](issues/P2-43-baumhard-seam-hygiene.md) · [44 bench discipline](issues/P2-44-bench-discipline.md) · [45 release logging policy](issues/P2-45-release-logging-policy.md) · [46 constants SSOT](issues/P2-46-constants-ssot.md) · [47 model serde + SectionRange](issues/P2-47-model-serde-polish.md) · [48 test gaps + small fixes](issues/P2-48-test-gaps-and-app-small-fixes.md) · [49 config-loader scaffolding](issues/P2-49-config-loader-scaffolding.md)

**P3:** [50 CONCEPTS sweep](issues/P3-50-concepts-accuracy-sweep.md) · [51 format/convention docs](issues/P3-51-format-and-convention-docs.md) · [52 doc-corruption sweep](issues/P3-52-doc-corruption-sweep.md) · [53 American English](issues/P3-53-american-english-sweep.md)

### Suggested sequencing

1. **Week-one safety:** P0-01…07, then P1-08/09/10/11 (walker + mutation contract), P1-20 (persisted corruption), P2-42 items 1–2.
2. **Unblock the pipeline work:** P1-12 before P1-22 step 1; P1-23 (turns every rebuild cheap) before the perf bundles.
3. **Structural consolidations, each shrinking the codebase:** P1-24/25/26/27/28/29/30, P2-40/41/49.
4. **Policy decisions (small diffs, need a human call):** P2-45 (release logging), P2-47-C (SectionRange meaning), P1-15 option a/b (palettes), P2-43-C (§B6 wire-or-reword).
5. **Sweeps last so they don't churn open work:** P3-50…53.

Cross-references between issues are noted inline where landing order matters (e.g. P1-12 ⇄ P1-22, P2-38 ⇄ P2-36, P1-26 ⇄ P1-15).

---

*Review artifacts: 10 area reports under `findings/` (each ending with its verified-clean list), `findings/verification-log.md` for the first-hand re-verification record. Baseline commit: `59cd115`. Suite: 2669 passed / 0 failed. The wasm32 type-check gate was environment-skipped locally (target not installed here) and is enforced by CI.*
