# P3-51: format/ spec + convention-doc reconciliation — text-run coverage rule inverted, TextRun optionality wrong, ControlPoint doc says the opposite of the code, TEST_CONVENTIONS says "No CI yet", macro-gate paragraph says gates are dormant

**Severity:** P3 (docs; several items are the authoritative author-facing contract) · **Area:** format/, CODE_CONVENTIONS, TEST_CONVENTIONS, CLAUDE.md

## format/ drift (docs claim authority; code wins where design is deliberate)

1. **text-runs.md teaches the inverse of the coverage rule**: "Uncovered ranges inherit the node's base style (so partial coverage is valid…)" with a worked example — but the implementation (by documented design elsewhere!) renders **only covered ranges**; uncovered graphemes drop. CONCEPTS contradicts itself within one entry, and `TextRun`'s own rustdoc says both things in one file (`node.rs:396-398` vs `:280-283`). Rewrite the coverage section + example to the drop rule; fix the rustdoc sentence; delete the "inherit" bullet from CONCEPTS. This manufactures exactly the "single biggest trap in the format" CONCEPTS warns about.
2. **TextRun optionality/type**: docs present bold/italic/underline/font/size_pt/color as optional; all are hard-required — the doc's own example fails to load (coordinate with P2-47's serde fix — fix code there, then docs here). `size_pt` typed "number" in schema.md but u32 in code.
3. **ControlPoint**: `edge.rs:568-572` rustdoc claims "Stored in canvas-space coordinates (absolute, not relative to the endpoints)" — every consumer treats them as **offsets from node centers** (`connection/mod.rs:119-134`), as do CONCEPTS and schema.md. Rewrite the rustdoc (and drop the nonsense "Copy-free only because f64 fields" sentence — f64 is Copy).
4. **schema.md**: Edge table omits `portal_from`/`portal_to`; six-vs-seven `target_scope` values (`SectionsOnly` missing, also in CONCEPTS).
5. **canvas.md vs schema.md disagree**: canvas.md lists `palettes` as a Canvas field (it's on MindMap) and omits `background_color`/`default_connection`/`theme_variants`.
6. **palettes.md**: "before wrapping" vs its own clamp rule (code clamps); `level` typed usize (code i32 — coordinate with P1-15).
7. **fonts.md** pins fonts at the pre-section `MindNode.text_runs[*].font` path.
8. **mutations.md** implies a scope helper per target_scope value; scope.rs ships four.
9. **validation.md** items tracked in P1-14 (grapheme wording, fill-parent, caps, zoom checks, new duplicate-edge check).
10. **migration.md** one-shot claim tracked in P1-13.

## Convention/orientation docs

11. **TEST_CONVENTIONS §T10 "No CI yet. ./test.sh is the covenant"** — two workflows exist and run on every push/PR (`.github/workflows/test.yml` enforces the wasm32 gate remotely). Rewrite the bullet; also fix test.yml's comment citing "CODE_CONVENTIONS.md §2" for dual-target discipline (it's §4), and §T11's "both crates" (three tested; four exist — coordinate with P2-40's workspace test fix).
12. **CODE_CONVENTIONS §3 macro-privilege paragraph**: "Today only the User tier loads, so the gates are dormant" — the code says the opposite (`macros/mod.rs:36-37,85-86`: "All four tiers load today on native; the gate is fully active."). This is the security-model paragraph of the contract document; update to the four-tiers-live reality.
13. **CODE_CONVENTIONS §4 references `./build.sh --wasm`** — the flag doesn't exist (build.sh accepts `--debug/--fat/--help`).
14. **TEST_CONVENTIONS §T8 points at "TODO.md's 'What needs work' list"** — actual path `work_plans/TODO.md`, actual section "Outstanding".
15. **CLAUDE.md**: wasm gate "fails the run" oversells the local soft-skip (CI enforces); `--fat` documented as working (broken until P2-40).
16. **work_plans/ hygiene**: SECTIONS_BORDERS_RESIZE_PLAN.md says "Under development" while Batches 2-8 are marked SHIPPED with 67 unchecked boxes (some inside SHIPPED batches) and references a nonexistent `REFACTOR_PLAN.md`; WASM_CONVERGENCE.md has a broken `./src/...` link and claims TODO.md tracks ~7 items (it tracks 1). Reconcile + archive the finished plan. In-code comments referencing the plan files by bare name (e.g. "SECTIONS_BORDERS_RESIZE_PLAN.md", "WASM_CONVERGENCE.md" without the work_plans/ path — several console/dispatch sites) should get correct paths or self-contained rationale.
17. **maptool USAGE/README/CLAUDE verb-surface sync**: verify's description omits two of its own check families; README/CLAUDE list only `convert --legacy` (three convert modes exist); CONCEPTS omits `--sections`.

## Acceptance criteria

- Every format/ claim spot-checked against a struct/loader test; docs and code agree on each numbered item (with code-first fixes where design says code wins).
- `rg "No CI yet|build.sh --wasm|cancel_mode" --type md` returns nothing stale.

## Pointers

format/*; CODE_CONVENTIONS.md; TEST_CONVENTIONS.md; CLAUDE.md; work_plans/; the model/maptool findings files for exact line evidence.
