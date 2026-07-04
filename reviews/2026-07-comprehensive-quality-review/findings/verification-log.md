# Verification log (my own first-hand checks)

## VERIFIED P0/P1
- F1 DeleteNode undo corruption: CONFIRMED. topology.rs:22 removes node BEFORE minting (line 35 fresh_child_id(None)); fresh_child_id (243-263) = max remaining root + 1, so deleting max-numbered root "N" with children re-mints "N" for orphan. undo.rs DeleteNode arm: nodes.insert(restored_id, node) silently overwrites renamed child; cascade_rename(root_id→old_id) then renames restored parent away; parent_id dangling. Trigger: delete max-numbered root with children, Ctrl+Z.
- F2 sync-back gap: CONFIRMED. grow-font-2pt = AreaCommand::GrowFont → tree area.scale; sync.rs:92 size_pt comes from PRIOR MODEL RUN (prior.map(|p| p.size_pt).unwrap_or(DEFAULT)) — tree scale never read back. Post-dispatch rebuild_all (click.rs:109, console exec.rs:98) reverts. Toggle: active_toggles has no re-apply after rebuild (grep confirmed by agent).
- F3 animation double-apply: CONFIRMED. animations.rs:371-373 lerps MODEL position per frame; :387 applies FULL custom mutation at completion on top of lerped state (relative mutations ≈ double delta); undo snapshot inside that call = mid-lerp baseline.
- R1 swapchain format: CONFIRMED. renderer/mod.rs:600 hardcodes Bgra8UnormSrgb (atlas line 611 + rect pipeline), :602 surface uses capabilities.formats[0]; comment at 617-621 self-contradictory ("matches the LoadOp target" — the LoadOp target uses formats[0]). Latent black-screen when formats[0] != Bgra8 (WebGL commonly Rgba8).
- B3 RepeatWhile ordering: CONFIRMED my own read (tree_walker.rs 181-241): m<t advances BOTH (drops matches even sorted); align_child_walks sorts, RepeatWhile+DEFAULT_TERMINATOR don't. Agent's [2,3]vs[1,2] example checks out.
- build.sh --fat broken: CONFIRMED empirically — cargo: "error: profile `release-lto` is not defined".
- syn + serde-lexpr phantom deps in mandala AND baumhard manifests: CONFIRMED (grep: syn only used by mandala_derive; lexpr zero hits).
- strum 0.27 (mandala) vs 0.28 (baumhard) divergence; no [workspace.dependencies]: CONFIRMED from manifests.
- CI exists (.github/workflows/test.yml runs ./test.sh) vs TEST_CONVENTIONS §T10 "No CI yet": CONFIRMED stale doc.
- test.sh: 2669 tests green; wasm32 gate SILENTLY SKIPS when target not installed (prints hint) — local covenant weaker than doc claims.
- rand dep in baumhard: get_some_font() pub test-only helper (fonts.rs:387-392) + stress-map bin. env_logger in baumhard lib deps (util/log.rs init()).
- log crate release_max_level_off in BOTH crates: all log::warn!/error! compiled out in release — §9's failure channel is silent in production builds. Design tension worth an issue.
- Convergent (input M11 + console #3): crate-wide #![allow(dead_code)] at src/main.rs:3 — confirmed by console agent via cargo check (zero warnings with confirmed-dead code).

## Notes
- mandala_derive: 538-line proc-macro crate, good SSOT design (ActionClassify). "defence" British spelling in doc.
- WASM_CONVERGENCE.md + TODO.md live in work_plans/ BUT console agent found code refs citing "SECTIONS_BORDERS_RESIZE_PLAN.md" and "WASM_CONVERGENCE.md" as if at repo root ("neither exists (verified ls *.md)") — they exist under work_plans/. Path drift in code comments, not missing files. Adjust console #18 accordingly when filing.
