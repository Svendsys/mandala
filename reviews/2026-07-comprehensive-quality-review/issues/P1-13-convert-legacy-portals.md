# P1-13: `maptool convert --legacy` output is rejected by the loader when the legacy map carried portals; convert writers are inconsistently atomic

**Severity:** P1 (documented one-shot migration contract broken; reproduced) · **Area:** maptool/convert

## Problem A — portals not folded into the legacy pipeline (reproduced)

`crates/maptool/src/convert/mod.rs:53-66` — `convert_legacy` runs `ids → enums → palettes → cleanup → sections` and even rewrites portal endpoint ids (`convert/ids.rs:109-115`), but never folds the legacy `portals[]` array into edges. `format/migration.md:44-48,92-96` promises "a single legacy hop produces a post-section file in one step" and "Run `maptool verify <output.json>` … It should exit 0".

Reproduced: legacy fixture with one portal → `convert --legacy` exits 0 → `verify` fails with `legacy 'portals' field present; run 'maptool convert --portals <file>'` (the loader rejects the file, so verify can't even parse it). Manually chaining `convert --portals` afterwards produces a verify-clean file, and every other transform checks out — the fix is composition, not new logic.

**Fix:** extract `convert_portals`'s Value-transform into a helper and call it inside `convert_legacy` (order: after ids, since portal endpoint ids must be rewritten consistently — verify against the existing `rewrite_portals` step). Add a fixture test: legacy-with-portals → `convert --legacy` → `verify` exits 0.

## Problem B — non-atomic writers

`convert --portals` writes via `write_atomic` (`convert/portals.rs:38,117`), whose doc (`loader.rs:170-175`) says it is exposed for exactly these tools. But `--legacy` (`convert/mod.rs:70`) and `--sections` (`convert/sections.rs:51`) use plain `fs::write` — while `main.rs:63-73` advertises in-place safety for both. `--sections` with input==output truncates the user's only copy on a mid-write kill. `sections.rs:10-12` even says "The pipeline mirrors `convert_portals`" — it mirrors everything but the atomic write.

**Fix:** route all three convert verbs through `write_atomic`; extract the read/parse/write scaffolding (currently pasted three times) into `convert/mod.rs` helpers.

## Acceptance criteria

- Legacy fixture with portals: `convert --legacy` then `verify` exits 0, one step.
- All convert verbs survive a simulated mid-write failure without destroying input (atomic temp+rename).
- `./test.sh` green; new fixture tests included.

## Pointers

`crates/maptool/src/convert/{mod.rs,portals.rs,sections.rs,ids.rs}`; `lib/baumhard/src/mindmap/loader.rs:170-175` (write_atomic); `format/migration.md`; CODE_CONVENTIONS §10 ("migration tooling kept in sync in the same commit").
