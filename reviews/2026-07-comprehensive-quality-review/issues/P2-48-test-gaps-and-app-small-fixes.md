# P2-48: Test gaps + small app-layer fixes — file-lifecycle verbs untested, loader-edge tests missing, WGSL lock-step unpinned; assorted verified one-liners

**Severity:** P2 (bundle of small, independent items) · **Area:** mandala + baumhard tests

## Test gaps

1. **File-lifecycle verbs have zero tests**: `open`, `new`, `save`, `fps` (`console/commands/{open,new,save,fps}.rs` — no `#[test]` anywhere, verified). The dirty-guard ("unsaved changes; save before opening") is the only thing between a typo'd `open` and losing unsaved work; save's rebind+clear-dirty semantics untested. All four are pure `ConsoleEffects` + `loader::save_to_file` — trivially testable with temp paths.
2. **Loader edges** (§T1 names them a fundamental): no test for malformed JSON through `load_from_str`, missing required node field, or unknown `edge_type` tolerance. Also `all_target_scopes_serialize` covers six variants while the enum has seven (`SectionsOnly` unpinned — the test name lies); `flat_mutations` has zero direct tests; loader tests write fixed-name files to `env::temp_dir()` (concurrent-run race — pid-suffix them); one test fixture is pre-section and only survives by bypassing the loader.
3. **WGSL↔Rust shape lock-step is comment-enforced only**: verified in agreement today (`SHAPE_RECT/ELLIPSE` vs `SHAPE_ID_*`, inclusive boundaries) — pin it with a plain-string test asserting `RECT_SHADER_WGSL` contains each `NodeShape` variant's constant (pure text, §T8-compatible).
4. **frame_throttle suite duplicates six scenarios under two naming schemes** (`frame_throttle.rs:156-349` vs `351-513`) — merge each pair keeping the stricter body; settle the `test_` prefix question per §T3 (several modules omit it).

## Small verified fixes (each ≤ ~15 lines)

5. `commit_border_preview` re-implements `edits_touch_cfg_field` inline (`nodes/border.rs:734-746` vs :925-934) — the exact one-field-drift the file already fixed once elsewhere (`document/mod.rs:774-781` precedent). Use the named predicate.
6. `SelectionState::is_selected` allocates a String per query for `Multi` (`types.rs:315` — `ids.contains(&node_id.to_string())`); called per candidate in hover/highlight paths. Use `iter().any(|i| i == node_id)`.
7. `set_*_font_size/_family` silently no-op on run-less sections — `.all()` on an empty iterator is vacuously true (`nodes/mod.rs:569-577,633-638`; `section_text.rs:343,384`; empty runs are a legal state). Create the default run at the requested size (coordinate with P1-26's `default_text_run`), or surface a distinguishable outcome.
8. `build_section_frames` warns on every non-drag rebuild in NodeEdit mode (`scene_builder/section_frame.rs:75-93`): the "invariant" it asserts doesn't exist — `offsets` is the *drag*-offsets map, empty by design in every non-drag build (`rebuild_scene_only` passes `&HashMap::new()`). Replace the warn with the `unwrap_or((0.0,0.0))` every sibling pass uses.
9. Parametric-Action failure feedback: `apply_border_field_to_selection` returns false on stage error with **no log** (`border/execute.rs:676-697`) — a dead keybinding users can't diagnose; siblings warn. Add the warn; extend `log_not_applicable_if_silent` to Invalid outcomes on the Action path.
10. Click-vs-drag hit priority is opposite for node-vs-portal (click: node beats portal, `app/mod.rs:260-315`; drag: portal beats node) and only the drag order matches CONCEPTS — same press, different target by gesture outcome. Align or document the split in CONCEPTS next to DragState.
11. maptool polish: verify prints the violation count twice (`main.rs:244-253` + `90-93`); Dewey ordering duplicated twice in main.rs and lexicographic for dotted ids (`"0.10"` before `"0.2"` — add `pub fn dewey_cmp` in baumhard model, use both sites); cycle reporting emits one violation per affected node (report each cycle once); verify modules clone a String per node to discard it at 4 sites.
12. WASM bundle ships ~1MB of unloadable legacy fixture via `copy-dir ../maps` (`web/index.html:15` — `testament.mind` is a 7z miMind archive with no Rust consumer). copy-file the three `.mindmap.json` fixtures instead; decide whether `testament.mind` belongs in-tree.
13. Scripts: `bench.sh` has no shebang/set-flags and duplicates test.sh's bench line; `debug_build.sh` is an undocumented one-line alias; `test.sh:69-70`'s count-grep under `set -euo pipefail` kills a green run with no message if cargo's output format shifts — guard the pipeline. Soften CLAUDE.md's "wasm type-check fails the run" to mention the local soft-skip (CI enforces).

## Acceptance criteria

- Each item lands with its own test where behavior changes; `./test.sh` green; no silent-failure paths remain in the listed set.

## Pointers

Files cited inline; TEST_CONVENTIONS §T1/§T3/§T8/§T12.
