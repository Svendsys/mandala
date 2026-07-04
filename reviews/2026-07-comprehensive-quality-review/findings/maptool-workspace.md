# Findings — maptool / scripts / workspace review (Mandala)

## Architecture assessment

The maptool crate is a genuinely well-built CLI: clear verb dispatch with distinct exit codes (usage=2, runtime=1), a sanctioned local `CliError` enum (correctly scoped as CLI posture, not a §9 violation), heavy inline test coverage (130 tests), deadlock-safe subprocess piping, atomic writes on `apply`, and section-aware regex targeting with regression tests pinning past corruption bugs. The verify/ module is cleanly decomposed (one invariant family per file) and the loader/verify division of labor — loader = syntax + legacy-shape rejection, verify = semantics — matches format/validation.md's stated design. The real problems are at the seams: the *content* of the checks has drifted from the format docs and from the app crate ("circle" rejected, channel-collision severity, code-points vs graphemes), the section-AABB validator is maintained byte-for-byte by hand in two crates instead of living in baumhard, the `convert --legacy` pipeline forgot to fold in the portals migration so its output can fail to load, and workspace hygiene has decayed (no workspace-level dependency table, strum compiled at two versions, dead `syn`/`serde-lexpr`/`futures` deps, an undefined `release-lto` profile that hard-breaks the documented `build.sh --fat`, and a fourth crate whose tests nothing runs). Docs are extensive but several load-bearing claims are stale — most notably TEST_CONVENTIONS' "No CI yet" while two GitHub workflows exist and CODE_CONVENTIONS references a `./build.sh --wasm` flag that doesn't exist.

## Verify-vs-loader invariant matrix (Quest 1)

| Invariant | validation.md claims | maptool verify | baumhard loader | app (document setters) | Drift? |
|---|---|---|---|---|---|
| JSON parses / typed shape | (implied) | via load | YES (serde) | — | clean |
| Legacy portals[] / node text / zero sections rejected | migration.md | — (loader gates first) | YES loader.rs:43-145 | — | clean |
| parent_id exists | YES | YES tree.rs:14-23 | no | render skips | intended split |
| No parent cycles | YES ("all_descendants loops forever") | YES tree.rs:27-42 | **no** | **crashes/hangs interactive paths** | **F4** |
| key == node.id | YES | YES ids.rs:14-19 | no | — | intended |
| derive_parent_id agrees / root no dot | YES | YES ids.rs:22-47 | no | — | intended |
| Edge from/to exist | YES | YES references.rs | no | render skips | intended |
| Palette exists / non-empty | YES | YES palettes.rs | no | falls back | intended |
| Named enums | YES (per enums.md) | YES but **shape list omits "circle"** | loader lenient; shape.rs:89 accepts circle case-insensitively | — | **F2** |
| text_runs start<end / no overlap | YES | YES | no | — | intended |
| run end ≤ text length | YES, in **code points** | YES, in **grapheme clusters** | no | — | **F14** (doc stale) |
| Section offset/size finite/positive | YES | YES sections.rs | no | duplicated validate_section_aabb | **F5** |
| Section AABB in node (incl. fill-parent) | says "When size is set" only | YES always via effective_size | no | duplicated | F14 |
| Node size finite/positive | YES | YES | no | duplicated validate_node_size | F5 |
| 100× section typo guard | YES | YES | no | duplicated | F5 |
| Node-size absolute ceiling (1e6) | not mentioned | **no** | no | YES check_node_size_typo (nodes/mod.rs:854-870) | **F5b** (verify gap) |
| Section channel collision | YES, "a *warning*, not a hard rejection" | hard violation, exit 1 | no | — | **F13** |
| Section count cap 1024 | **not documented** | YES (local const) | **no** (cap claims "OOM at load" defense but load precedes verify) | YES add_section (own const) | **F12** |
| Zoom min ≤ max | YES | YES | no | validate_zoom_pair mirrors | clean |
| Zoom non-finite | **not documented** | YES zoom_bounds.rs:78-95 | no | mirrors | F14 |
| trigger_bindings.mutation_id resolves | documented as NOT checked | not checked | no | — | clean (honest) |
## Findings

### F1. `convert --legacy` output is rejected by the loader when the legacy map carried portals
Severity: P1 | Category: correctness | Confidence: high
Files: crates/maptool/src/convert/mod.rs:53-66; crates/maptool/src/convert/ids.rs:109-115; format/migration.md:44-48,92-96; lib/baumhard/src/mindmap/loader.rs:96-104
Evidence: `convert_legacy` pipeline runs `ids → enums → palettes → cleanup → sections` and even rewrites portal endpoints (`rewrite_portals`), but never folds `portals[]` into edges. migration.md promises: "runs automatically inside `convert --legacy` … so a single legacy hop produces a post-section file in one step" and "Run `maptool verify <output.json>` … It should exit 0". Reproduced: synthetic legacy fixture with one `portals[]` entry → `convert --legacy` exits 0 → `verify` fails with `legacy 'portals' field present; run 'maptool convert --portals <file>'` (the loader rejects the file, so verify can't even parse it). After manually chaining `convert --portals`, verify exits 0 and every transform (Dewey IDs by index, integer→named enums, palette hoist, channel default, section fold) is correct.
Why it matters: The migration tool's one-shot contract (§10 "migration tooling kept in sync") is broken for exactly the class of map the `rewrite_portals` pass proves it expects. A user follows migration.md verbatim and gets a file Mandala refuses to load.
Fix: Call the portals fold (extract the body of `convert_portals` into a `Value`-level `migrate_portals(root)` helper) inside `convert_legacy` between cleanup and sections; add a legacy-fixture test asserting `verify(convert_legacy(x)) == clean`.
Effort: S

### F2. `maptool verify` rejects the documented-valid shape `"circle"`
Severity: P1 | Category: correctness | Confidence: high
Files: crates/maptool/src/verify/enums.rs:9-16; format/enums.md:27-45; lib/baumhard/src/gfx_structs/shape.rs:89-93; format/validation.md:55-66
Evidence: `SHAPES` = `["rectangle","rounded_rectangle","ellipse","diamond","parallelogram","hexagon"]` — no `"circle"`. format/enums.md lists `"circle"` among "The known values" ("the convenience spelling authors reach for first"); shape.rs:89 accepts it (`eq_ignore_ascii_case("circle")` → Ellipse). Reproduced: theme_demo with `shape:"circle"` → `enums @ 0: style.shape "circle" is not one of [...]`, exit 1. validation.md defers to enums.md "for the complete lists".
Why it matters: verify emits false violations on files the format spec and renderer both accept — the tool users are told to trust flags valid maps as broken. Side observation: shape.rs matches case-insensitively while verify is case-sensitive (`"Rectangle"` renders, fails verify) — decide and document which is canonical.
Fix: Add `"circle"` to SHAPES; better, export the canonical name list (and the other enum sets) as `pub const` slices from baumhard's model (DISPLAY_MODE_LINE/PORTAL already exist but verify hardcodes `"line"`/`"portal"` strings instead of using them) and have verify, shape.rs docs, and format/enums.md generation all reference one source.
Effort: S (list fix) / M (SSOT constants)

### F3. `./build.sh --fat` is hard-broken: profile `release-lto` is not defined
Severity: P1 | Category: correctness | Confidence: high
Files: build.sh:55-58,83; Cargo.toml (root — no `[profile.*]` sections); CLAUDE.md:68 ("--fat switches native to release-lto"); no .cargo/config.toml exists
Evidence: Reproduced: `cargo build --profile release-lto -p maptool` → `error: profile 'release-lto' is not defined`. build.sh also `rm -rf target/release-lto dist` *before* invoking cargo, so the failing run still deletes the prior WASM bundle.
Why it matters: A documented release flag (CLAUDE.md "Common tasks") fails on first use; the preflight checks trunk/wasm32 but not the profile, and the cleanup-before-build ordering destroys prior output on the failure path.
Fix: Define `[profile.release-lto] inherits = "release", lto = "fat"` (plus e.g. `codegen-units = 1`) in the root Cargo.toml; keep the doc comment explaining when to use it.
Effort: S

### F4. Cyclic `parent_id` chains load fine and then crash/hang interactive paths
Severity: P1 | Category: correctness | Confidence: high (mechanics) / med (real-world trigger frequency)
Files: lib/baumhard/src/mindmap/model/mod.rs:134-148 (is_hidden_by_fold — unguarded loop), 151-162 (all_descendants — unguarded recursion), 167-179 (is_ancestor_or_self — unguarded loop); lib/baumhard/src/mindmap/loader.rs (no cycle check); consumers: src/application/app/throttled_interaction/moving_node.rs:117, src/application/document/topology.rs:289, src/application/document/custom/mod.rs:569-572
Evidence: `collect_descendants` recurses via `children_of` with no visited set — for `a→b→a` it recurses forever (stack overflow). `is_hidden_by_fold` walks the parent chain with no guard — infinite loop during scene build. validation.md:23 itself states "A cycle makes `all_descendants` loop forever". Only `maptool verify` (tree.rs:27-42) detects cycles; the app never runs it, and the loader accepts cyclic files.
Why it matters: §9 "Interactive paths must not panic" — a hand-edited `.mindmap.json` (hand-authoring is a design goal; the entire format/ dir exists for it) loads, then the first drag/delete/fold touching the cycle crashes or hangs the app. Degrading the frame is impossible once you're inside unbounded recursion.
Fix: Either reject cycles at load (loader-side walk, mirroring tree.rs) or add visited-set guards to the three walkers in model/mod.rs (cheap: HashSet of &str, log::warn! and bail). Guarding the model helpers also fixes maptool `export` silently dropping cycle members.
Effort: S-M

### F5. Section/node bounds validation duplicated byte-for-byte across app and maptool instead of living in baumhard
Severity: P1 | Category: duplication | Confidence: high
Files: crates/maptool/src/verify/sections.rs:63-309 (check_node_size_finite, check_offset_finite/non_negative, check_size_finite/positive, check_size_not_astronomical, check_within_node_aabb) ⇔ src/application/document/nodes/mod.rs:747-845 (validate_node_size, validate_section_aabb); format/sections.md:147-152 and format/validation.md:100-108 ("Rejection messages are byte-equal")
Evidence: Both sites implement the identical rule set with identical format strings, e.g. `"section[{}].offset.x is negative ({})"`, `"section[{}] extends past node right edge ({} > {})"`, `"...is over 100× the node's width ({}); likely a typo (e.g. an extra zero)"`. The docs *celebrate* the byte-equality, but it's maintained by hand in two crates; only `MindSection::effective_size` is shared. Drift has already begun (F5b): the app additionally enforces `MAX_NODE_AXIS = 1_000_000.0` (nodes/mod.rs:854-870) — verify has no node-size ceiling, so a file with `node.size.width = 1e30` passes verify while every app setter refuses to produce it.
Why it matters: §5 "If a function is needed in two or more places, the answer is never to copy it"; §1 "Missing primitives are added to Baumhard". Any wording tweak or new rule must be mirrored manually, and the byte-equal doc claim silently rots.
Fix: Move the checks into baumhard (e.g. `mindmap::model::section_bounds`), returning either `Result<(), String>` (app) with a thin all-violations iterator wrapper (verify). Add the node-ceiling to verify in the same move and document it in validation.md.
Effort: M

### F6. TEST_CONVENTIONS §T10 says "No CI yet" — two GitHub workflows exist and run on every push/PR
Severity: P2 | Category: docs | Confidence: high
Files: TEST_CONVENTIONS.md:240-241 ("**No CI yet.** `./test.sh` is the covenant"); .github/workflows/test.yml (runs ./test.sh with wasm32 target installed); .github/workflows/license-headers.yml; CODE_CONVENTIONS.md:213-214 (correctly cites CI); .github/workflows/test.yml:4 (comment cites "CODE_CONVENTIONS.md §2" — dual-target discipline is §4, §2 is "integration tasks")
Evidence: test.yml: `on: push: branches: [main], pull_request:` → `run: ./test.sh` with `targets: wasm32-unknown-unknown`. So CI exists and *does* enforce the WASM gate (locally test.sh soft-skips when the target is missing — observed in this environment: suite green, gate skipped with a warning).
Why it matters: The two convention docs contradict each other on a "do not re-litigate" decision item; §T10 is the stale one. The wrong section pointer in test.yml misleads readers.
Fix: Rewrite §T10's bullet ("CI runs ./test.sh + SPDX check; ./test.sh remains the pre-commit covenant"); fix test.yml's comment to cite §4.
Effort: S

### F7. CODE_CONVENTIONS §4 references `./build.sh --wasm` — the flag does not exist
Severity: P2 | Category: docs | Confidence: high
Files: CODE_CONVENTIONS.md:212-214; build.sh:14-25 (accepts only --debug/--fat/--help; unknown args exit 1)
Evidence: "`./test.sh`'s WASM type-check gate, `./build.sh --wasm`, and CI (`.github/workflows/test.yml`) enforce this." Running `./build.sh --wasm` prints `Unknown argument: --wasm` and exits 1.
Why it matters: A mandatory-read convention doc tells contributors to run a command that errors; the actual behavior (build.sh always builds both targets) is better than the doc implies.
Fix: Change to "`./build.sh` (always builds both targets)".
Effort: S

### F8. mandala_derive's 13 tests are never run by test.sh or CI
Severity: P2 | Category: testing | Confidence: high
Files: test.sh:67 (`cargo test -p baumhard -p mandala -p maptool`); lib/mandala_derive/src/lib.rs (13 `#[test]` fns); Cargo.toml:2 (workspace has 4 members); CLAUDE.md:59-60 and TEST_CONVENTIONS.md §T11 ("across both crates" / "across `baumhard` and `mandala`" — also stale re maptool)
Evidence: The proc-macro crate is a workspace member with a test module, but the explicit `-p` list omits it, so neither local runs nor CI ever execute those tests. The docs still say "both crates" while three are tested and four exist.
Why it matters: §11 "extensive unit testing is a directive" — a crate's suite silently never runs; a regression in the derive macro would surface only as downstream compile errors, not as its own test failures.
Fix: Use `cargo test --workspace` (also future-proofs new crates), and update CLAUDE.md/TEST_CONVENTIONS wording to name the workspace.
Effort: S

### F9. No `[workspace.dependencies]`; versions hand-synced across three manifests and already diverged (strum 0.27 vs 0.28 both compiled)
Severity: P2 | Category: ssot | Confidence: high
Files: Cargo.toml:34-35 (strum/strum_macros "0.27"); lib/baumhard/Cargo.toml:25-26 ("0.28.0"); duplicated pins across Cargo.toml, lib/baumhard/Cargo.toml, crates/maptool/Cargo.toml for serde 1.0.228, serde_json 1.0.149, serde-lexpr 0.1.3, lazy_static 1.5.0, syn 2.0.117, log 0.4.29, smol_str 0.3.6, indextree 4.8.1, glam 0.32.1, cosmic-text 0.18.2, tinyvec 1.11.0, unicode-segmentation 1.13.2, rustc-hash 2.1.2, criterion 0.8.2, regex 1.12.3, walkdir/path-slash/ttf-parser
Evidence: `cargo tree -d` shows `strum v0.27.2` + `strum v0.28.0` and `strum_macros` at both versions (two proc-macro compiles) — caused directly by the intra-workspace divergence, not by third parties. Remaining duplicates (smol_str 0.2/0.3, rustc-hash 1/2, syn 1/2, thiserror 1/2, getrandom 0.3/0.4, itertools, hashbrown, foldhash, rustix) are transitive ecosystem noise.
Why it matters: §5 duplication; the strum drift proves hand-syncing has already failed once. Every shared bump is a 2-3 file edit that can silently fork.
Fix: Add `[workspace.dependencies]` for all shared deps, switch member manifests to `dep.workspace = true`, unify strum on 0.28.
Effort: M
### F10. Dead dependencies: `syn` (2 crates), `serde-lexpr` (2 crates), `futures`, and a root `[build-dependencies]` block with no build.rs
Severity: P2 | Category: dead-code | Confidence: high
Files: Cargo.toml:25 (futures), :29 (serde-lexpr), :31 (syn), :46-51 ([build-dependencies] walkdir/path-slash/ttf-parser/regex/lazy_static — no build.rs exists at repo root); lib/baumhard/Cargo.toml:12 (serde-lexpr), :15 (syn)
Evidence: `grep -rn "use syn|syn::"` over src/ and lib/baumhard/src/ → zero hits (only lib/mandala_derive legitimately uses syn). `serde_lexpr`/`lexpr` → zero hits. `futures::` in src/ → zero hits (WASM async uses `wasm_bindgen_futures`, native uses `pollster`). Root has no build.rs (`ls build.rs` → not found), so its five build-deps are never compiled for a build script — the block appears copy-pasted from baumhard (which does have build.rs and uses them).
Why it matters: §5 "no dead code". `syn` is among the heaviest compile-time crates and is pinned as a *runtime* dep of both the app and the glyph library; the dead manifest entries mislead every dependency audit.
Fix: Delete `syn`, `serde-lexpr`, `futures` from Cargo.toml and lib/baumhard/Cargo.toml; delete the root `[build-dependencies]` section.
Effort: S

### F11. `crossbeam-channel`'s only production purpose is an unused `_scene_index_sender` parameter on `Tree::new`
Severity: P2 | Category: dead-code, api-design | Confidence: high
Files: lib/baumhard/src/gfx_structs/tree.rs:18,179 (`pub fn new(region_params: Arc<RegionParams>, _scene_index_sender: Sender<RegionElementKeyPair>)`); lib/baumhard/Cargo.toml:29; only other use: gfx_structs/tests/tree_tests.rs:1036 (`let (mock_sender, _mock_receiver) = unbounded();`)
Evidence: The parameter is underscore-ignored; all production call sites use `Tree::new_non_indexed()`; the test constructs a channel solely to satisfy the dead signature. §3: "No channels, no worker threads … in interactive paths" — the dependency exists to type a parameter that contradicts the architecture doc and is never read.
Why it matters: A `pub` baumhard constructor (documented API per §B9) demands a cross-thread channel that does nothing; the dependency implies a threading story the project explicitly forbids.
Fix: Remove the parameter (or the whole constructor if `new_non_indexed` + indexed variant cover the seam) and drop crossbeam-channel from baumhard. If a scene-index event seam is the named trajectory, keep the seam via a trait/callback, not a dead channel.
Effort: S-M

### F12. MAX_SECTIONS_PER_NODE=1024 duplicated, and verify's OOM-defense rationale is impossible as written
Severity: P2 | Category: ssot, docs | Confidence: high
Files: src/application/document/mod.rs:104-106 (`pub const MAX_SECTIONS_PER_NODE: usize = 1024`); crates/maptool/src/verify/sections.rs:97-113 (local `const MAX_SECTIONS_PER_NODE: usize = 1024` + comment "would OOM at load … checked at every entry point"); lib/baumhard/src/mindmap/loader.rs (no cap); format/validation.md (cap not documented)
Evidence: Two hand-synced constants in two crates (maptool cannot import the app's; the natural home, baumhard, has none). The verify docstring claims the check defends against "a `\"sections\": [{},{},…10M…]` JSON payload [that] would OOM at load" — but verify itself must load the file first (`load_map` → serde) and the loader/serde allocates everything before the check runs, so the defense can never fire before the OOM it names. The one entry point that could defend (the loader) has no cap. The check is also absent from validation.md's "What gets checked".
Why it matters: §5 duplication + a misleading safety claim on a hostile-input defense; if the app cap changes, verify silently drifts.
Fix: Move the constant to baumhard's model; reference it from both; either enforce it in the loader (where the OOM claim would become true) or reword the verify docstring to "flags maps exceeding the app's add_section cap"; document the check in validation.md.
Effort: S

### F13. Channel-collision check: validation.md promises a "warning", implementation hard-fails verify
Severity: P2 | Category: docs, correctness | Confidence: high
Files: format/validation.md:96-100 ("Surfaced as a *warning*, not a hard rejection … authors who deliberately want broadcast can ignore it"); crates/maptool/src/verify/sections.rs:124-151 (pushes a regular Violation); crates/maptool/src/main.rs:235-255 (any violation ⇒ exit 1); crates/maptool/src/verify/mod.rs:22-27 (Violation has no severity field)
Evidence: There is no warning tier anywhere — `Violation` is category+location+message, and `run()` exits nonzero if the vec is non-empty. An author deliberately using channel broadcast cannot get exit 0, so the documented "can ignore it" workflow (and the CI recipe in validation.md:143-149) is impossible for such maps.
Why it matters: The verify exit code is the format's advertised CI contract; docs and behavior disagree on whether a legal authoring pattern fails it.
Fix: Either add a `severity: Warning|Error` field to Violation (print warnings, exit 0 when only warnings) or change validation.md to say collisions are hard violations. The former matches the documented intent.
Effort: S-M

### F14. format/validation.md stale in four places vs the implemented checks
Severity: P2 | Category: docs | Confidence: high
Files: format/validation.md:80 ("`end` is within the text's **code-point count**") vs crates/maptool/src/verify/text_runs.rs:3-5,21,42 (grapheme clusters; its own tests assert the message "must drop the old unit"); validation.md:87-90 ("When `size` is set: … AABB inside the parent") vs sections.rs:44-45 (AABB checked for `None`-size fill-parent sections too, via effective_size — schema.md:155 already documents the new behavior); validation.md (section-count cap absent, see F12); validation.md:112-119 (zoom bounds documents only min≤max) vs zoom_bounds.rs:78-95 (also flags non-finite bounds)
Why it matters: validation.md is the authoritative statement of what verify enforces (CONCEPTS.md §6 says so explicitly); each stale cell misleads map authors and future verify contributors. text-runs.md:46-47 already has the correct grapheme wording — validation.md lags it.
Fix: Update the four passages; add the cap and non-finite-zoom bullets.
Effort: S

### F15. Loader's "cheap" legacy screen false-positives on every current map with text runs → full second JSON parse per load
Severity: P2 | Category: performance | Confidence: high
Files: lib/baumhard/src/mindmap/loader.rs:83-85 (`has_legacy_marker`: `json.contains("\"text_runs\":")`), 53-59 (triggers `detect_legacy_shape`), 92-93 (re-parses whole file to `serde_json::Value`)
Evidence: Post-refactor files legitimately contain `"text_runs":` inside `sections[]` (testament.mindmap.json does, 557 KB), so the marker matches nearly every real map, and `detect_legacy_shape` re-parses the entire document a second time on every load. The function doc (loader.rs:36-42) reasons carefully about `":` suffixes preventing false positives from node *text*, but misses that the current format's own section keys emit the identical byte pattern. Startup-path cost, self-described as "Felt every map load".
Why it matters: The screen exists purely to keep the expensive `Value` walk off the happy path; as written it puts the walk *on* the happy path for any styled map — 2× parse per load on desktop and mobile WASM (§4 mobile budget).
Fix: Scope the marker to node-level keys only (e.g. run the screen against `nodes.*` values via the typed map: `sections` non-empty already proves post-refactor; only screen for `"portals":`), or drop the text_runs marker and rely on the zero-sections symptom + portals marker.
Effort: S

### F16. convert writers inconsistent: `--portals` atomic, `--legacy`/`--sections` plain `fs::write`
Severity: P2 | Category: error-handling, duplication | Confidence: high
Files: crates/maptool/src/convert/portals.rs:11,38,117 (uses `write_atomic`); crates/maptool/src/convert/sections.rs:51 and convert/mod.rs:70 (`std::fs::write`); crates/maptool/src/main.rs:63-64,71-73 (usage text says input/output "may be the same file" for both --portals and --sections); lib/baumhard/src/mindmap/loader.rs:170-175 (write_atomic doc: "exposed for legacy-migration tools (`maptool convert --portals` etc.)")
Evidence: All three converters share the read→transform→pretty-print→write pipeline (sections.rs:10-12 even says "The pipeline mirrors `convert_portals`"), yet two of three skip the atomic helper. For `--sections` with input == output (explicitly advertised), a mid-write kill truncates the only copy of the user's map.
Why it matters: §2 "repetition of *idiom* … is the point" — the crash-safety idiom exists, is exported for exactly this purpose, and is applied to one of three identical pipelines. Also duplication: the read/parse/serialize/write scaffolding is pasted three times.
Fix: Route all three through `write_atomic` (and optionally extract a shared `read_value / write_value_atomic` pair in convert/mod.rs).
Effort: S

### F17. Shipped keybinds template uses `cancel_mode` — the schema field was renamed `exit_mode`, so the template key is silently ignored
Severity: P2 | Category: correctness, docs | Confidence: high
Files: config/default_keybinds.json:8 (`"cancel_mode": ["Escape"]`); src/application/keybinds/config.rs:63,300,527 (`exit_mode`, `Action::ExitMode`); no `deny_unknown_fields` on KeybindConfig (config.rs:39-41), so unknown keys vanish silently; work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md:2384 records the rename
Evidence: Only occurrence of `cancel_mode` in the repo is the template. A user who copies the file per its own `_comment` instructions and edits that entry gets no effect and no warning.
Why it matters: The template is the documented customization entry point (XDG copy / `--keybinds` / `?keybinds=`); shipping a dead key breaks the first thing a rebinding user touches. §10: rename means rename everywhere, same commit.
Fix: Rename the key to `exit_mode`; consider a startup `log::warn!` for unrecognized keybind keys to catch future drift.
Effort: S

### F18. Orphaned/mis-attached doc comments and a duplicate `#[test]` attribute on load-bearing items
Severity: P2 | Category: docs, dead-code | Confidence: high
Files: src/application/document/nodes/section_structure.rs:708-720 (doc for the node-size-undo test + `#[test]` at 714, then a second doc + second `#[test]` at 720, all attached to `fn add_section_rejects_at_cap`; the described test actually lives at :744); src/application/document/nodes/mod.rs:720-746 (three stacked unrelated doc comments — validate_zoom_pair's guard doc, a text_runs-clamp doc, and the verify-parity doc — all attached to `fn validate_node_size` at :747; validate_zoom_pair at :886 has no doc)
Evidence: `#[test]` appears twice on one function (compiles, but trips the `duplicate_macro_attributes` future-compat lint); the stranded doc comments describe functions that live elsewhere, so `cargo doc` and readers get wrong contracts on the validators.
Why it matters: §8 documentation discipline — these are exactly the validators whose byte-equal messages the format docs depend on (F5); wrong attached docs on them is compounding. §5: every merge is a state we would ship.
Fix: Re-home the three doc comments to their functions; delete the stray `#[test]` + orphaned doc block above `add_section_rejects_at_cap`.
Effort: S

### F19. work_plans/ hygiene: finished plan not archived, contradictions, dangling references
Severity: P2 | Category: docs, dead-code | Confidence: high
Files: work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md:3 ("Status. Under development") vs :2332-2738 (Batches 2-8 all "— SHIPPED") with 67 unchecked `[ ]` boxes remaining, many *inside* SHIPPED batches (e.g. :2374-2393 under Batch 2); :8 and :2765 reference `REFACTOR_PLAN.md` which does not exist anywhere in the repo; work_plans/WASM_CONVERGENCE.md:14 (broken relative link `./src/application/app/run_wasm/` — doc lives in work_plans/, siblings use `../`); WASM_CONVERGENCE.md:133 ("deferred today (and tracked in TODO.md)" lists ~7 items) vs work_plans/TODO.md (tracks exactly 1: WASM filesystem); TEST_CONVENTIONS.md:197-198 says to note renderer bugs "in `TODO.md`'s 'What needs work' list" — the file's section is titled "Outstanding" and lives at work_plans/TODO.md, not repo root
Verdicts: SECTIONS_BORDERS_RESIZE_PLAN.md — shipped; archive or delete (reconcile the 67 boxes first: mark superseded or extract to TODO.md). WASM_CONVERGENCE.md — live and accurate on the code (Tracks B/C landed, cross_dispatch exists); fix the link and the TODO.md claim. TODO.md — live; align §T8's pointer (path + section name) or retitle the section.
Why it matters: §5 "no dead docs"; a contract-style plan that says "Under development" atop eight SHIPPED batches actively misleads the next session that's told to execute against it.
Effort: S-M
### F20. 588 British spellings across 169 files — project mandates American English
Severity: P3 | Category: convention | Confidence: high
Files (representative; duplication shown at all in-scope sites): crates/maptool/src/main.rs:351,670 ("recognised"); crates/maptool/src/verify/sections.rs:39 ("honours"), :550 ("honoured"); crates/maptool/src/convert/sections.rs:150 ("serialised"); run.sh:5 ("artefact"), :55 ("optimisation"); build.sh:38 ("unoptimised"); rustfmt.toml:25 ("behaviour"); CONCEPTS.md (dozens: "colour", "coloured", "serialised", "behaviour"); lib/baumhard/src/mindmap/loader.rs:664 ("randomised"), :747,858 ("serialised"); plus src/application throughout
Evidence: `grep -riE '\b(colour|behaviour|recognised|serialised|...)\b'` → 588 hits in 169 files.
Why it matters: CLAUDE.md §6 "Use American English for consistency"; §2 "NEVER skip changes because they are merely cosmetic".
Fix: One mechanical sweep (comments/docs/prose only — verify no string literals are load-bearing first; the verify test at text_runs.rs asserts on "grapheme clusters" wording, so re-run tests after).
Effort: M (bulk, mechanical)

### F21. CLI/docs disagree on the verb surface; verify USAGE omits two of its own check families
Severity: P3 | Category: docs | Confidence: high
Files: crates/maptool/src/main.rs:74-80 (verify described as checking "parent_id consistency, Dewey IDs, edge and portal references, palette references, named enums, text-run bounds" — omits section bounds and zoom bounds, both implemented in verify/mod.rs:66-77); README.md:71-73 (lists only `convert --legacy`); CLAUDE.md:48-51 (only `--legacy`); CONCEPTS.md:2572-2577 (`--legacy`, `--portals` — omits `--sections`)
Why it matters: Three docs each advertise a different subset of the actual five convert/verify capabilities; the tool's own --help is the worst offender for verify.
Fix: Update USAGE and the three doc sites to the full verb/flag set.
Effort: S

### F22. verify output prints the violation count twice; cycle reporting is noisy
Severity: P3 | Category: error-handling | Confidence: high
Files: crates/maptool/src/main.rs:244-253 (`eprintln!("{} violation(s)", …)` followed by `Err(CliError::NotFound(format!("{} violation(s) in {}", …)))` which main.rs:90-93 prints again — observed live: "1 violation(s)" then "1 violation(s) in <path>"); crates/maptool/src/verify/tree.rs:27-42 (every node whose ancestor chain touches a cycle emits its own violation — a 2-cycle under a deep subtree floods output)
Fix: Drop the pre-print and let the returned error carry the single summary; optionally report each cycle once (flag only nodes *inside* the cycle, or dedupe by cycle set).
Effort: S

### F23. Dewey ordering duplicated twice in main.rs and lexicographically wrong for dotted IDs
Severity: P3 | Category: duplication, correctness | Confidence: high
Files: crates/maptool/src/main.rs:331-334 (grep_nodes sort) and :438-441 (select_section_targets sort) — identical `parse::<u64>` numeric-else-lexicographic closures; lib/baumhard/src/mindmap/model/mod.rs:197-202 (`id_sort_key` — last-segment only, also not a full Dewey comparator)
Evidence: `"0.10".parse::<u64>()` fails, so dotted IDs compare lexicographically: `0.10` sorts before `0.2`. Only root-level pure-integer IDs get numeric order.
Why it matters: §5 copy-paste (same closure twice); output ordering surprises on maps with ≥10 children. Baumhard is the natural home for a segment-wise Dewey comparator (§1) — export.rs already leans on `id_sort_key` for sibling order and would benefit too.
Fix: Add `pub fn dewey_cmp(a: &str, b: &str) -> Ordering` (segment-wise numeric) to baumhard model; use it at both maptool sites.
Effort: S

### F24. WASM bundle ships 1 MB of unloadable legacy fixture + docs via copy-dir
Severity: P3 | Category: performance | Confidence: high
Files: web/index.html:15 (`<link data-trunk rel="copy-dir" href="../maps" />`); maps/testament.mind (1,037,034 bytes — a 7z miMind archive nothing in the repo can read; maps/docs/mimind-format.md documents it, no Rust consumer exists); maps/docs/*.md
Evidence: copy-dir copies the whole directory into dist/, so every web deploy includes the 1 MB .mind archive and format docs alongside the three loadable .mindmap.json files.
Why it matters: §4 mobile budget — dead megabyte per deploy; testament.mind itself is also questionable repo payload (its converted JSON is the canonical fixture; git history preserves the source).
Fix: copy individual .mindmap.json files (multiple `rel="copy-file"` links) or move fixtures the web build should serve into a dedicated dir; decide whether testament.mind belongs in-tree at all.
Effort: S

### F25. Script robustness nits: bench.sh bare, debug_build.sh redundant, test.sh count-pipeline trap, duplicated bench invocation
Severity: P3 | Category: convention, duplication | Confidence: high
Files: bench.sh (single line `cargo bench -p baumhard -p mandala` — no shebang, no `set -euo pipefail`, duplicates test.sh:77's --bench branch verbatim; omits maptool consistent w/ no benches, but also omits nothing else); debug_build.sh (`#!/bin/bash` + `./build.sh --debug ` — one-line alias, only script not using `#!/usr/bin/env bash`, undocumented in CLAUDE.md); test.sh:69-70 (`TOTAL=$(grep -E '^test result: ok\. …' | awk …)` — under `set -euo pipefail` a grep miss (cargo output format change) makes the assignment fail and kills an otherwise-green run with no message); test.sh:91-98 (wasm gate soft-skips when target missing — sensible, but CLAUDE.md:60-61's "type-checks wasm32-unknown-unknown so cross-platform drift fails the run" is unconditional; observed skipped-and-green in this environment)
Fix: give bench.sh a shebang+set flags or delete it and bless `./test.sh --bench`; delete debug_build.sh; `TOTAL=$(… || true)` or `grep -c`-style guard; soften the CLAUDE.md sentence ("fails the run when the target is installed; CI always installs it").
Effort: S

### F26. verify modules clone a String per node just to discard it at 4 of 6 call sites
Severity: P3 | Category: performance | Confidence: high
Files: lib/baumhard/src/mindmap/model/mod.rs:101-103 (node_locations clones `n.id` per node); discarded at crates/maptool/src/verify/tree.rs:13,27, palettes.rs:13, text_runs.rs:15, sections.rs:26 (`for (_loc, node) in map.node_locations()`); used at enums.rs:27 and zoom_bounds.rs:18
Why it matters: Trivial for a CLI, but it's the exact allocation-discipline smell §B1/§4 police; `map.nodes.values()` is the direct spelling at the four discarding sites.
Fix: Iterate `values()` where the stamp is unused.
Effort: S

### F27. mandala-side criterion template can never run as criterion; conventions say the harness shouldn't exist at all
Severity: P3 | Category: testing, docs | Confidence: med
Files: benches/_template.rs (criterion_group/criterion_main); Cargo.toml (no `[[bench]] harness = false` for it — contrast lib/baumhard/Cargo.toml:46-48); Cargo.toml:65-66 (criterion dev-dep on mandala); test.sh:77 + bench.sh (`cargo bench -p mandala`); TEST_CONVENTIONS.md §T2.3 ("The `mandala` crate has no benchmark harness")
Evidence: Auto-discovered bench target `_template` compiles under the default libtest harness, which ignores `criterion_main!`'s `fn main` and runs 0 benchmarks — silently. Anyone copying the template for a real mandala bench inherits the same silent no-op unless they also add the missing manifest section. Meanwhile §T2.3's premise ("no benchmark harness") contradicts the dev-dep + template + bench invocations.
Fix: Either delete the template + criterion dev-dep + `-p mandala` from bench invocations (align with §T2.3), or add `[[bench]] name = "_template" harness = false` and a comment in the template about the manifest requirement.
Effort: S

### F28. .gitignore redundancy
Severity: P3 | Category: convention | Confidence: high
Files: .gitignore:4 (`/target/llvm-cov` — already covered by line 1 `/target`)
Fix: Drop the redundant line.
Effort: S

## Checked and CLEAN

- assets/mutations/application.json — validates against format/mutations.md (legacy `mutations`+`target_scope` shape, contexts `map.node`/`map.tree` documented); handler-backed ids `flower-layout`/`tree-cascade` exactly match `register_builtin_handlers` (src/application/document/mutations/mod.rs:38-42); the three pure-data ids need no handlers; a startup-parse test exists (mutations_loader/builtin.rs:35). assets/macros/application.json is a valid empty array.
- All shipped maps verify clean: testament / theme_demo / stress_long_edges → exit 0; testament loads (252 nodes / 258 edges, pinned by loader tests).
- Full `./test.sh` run in this environment: 2669 tests, 0 failures (wasm gate soft-skipped, see F25; CI installs the target so the gate holds remotely).
- Two-step legacy migration correctness: synthetic miMind-style fixture → `convert --legacy` → `convert --portals` → verify exit 0; Dewey assignment honors `index` order, enums/palette-hoist/channel/cleanup/section-fold all correct (only the missing one-shot chaining, F1).
- maptool `apply`: atomic write (temp+rename via save_to_file) as advertised; section-index routing correct with regression tests pinning the old sections[0] corruption; run_pipe is deadlock-safe (writer thread, 256 KiB test), swallows EPIPE, enforces UTF-8 output, strips exactly one trailing newline incl. CRLF; --dry-run's side-effect caveat is documented; unknown `--flags` rejected; zero matches exits 1 with no write; subprocess failure leaves the file untouched (all-or-nothing).
- maptool error posture: no unwraps outside `mod tests`; single `expect` guards a locally-proven invariant; no TODO/FIXME/HACK markers.
- verify enum sets other than shape match format/enums.md exactly (layout types, directions, line styles, anchors, edge types, display modes).
- export.rs: builds an O(N) child index with a documented rationale for bypassing `children_of` (O(N²)); empty-text passthrough, notes/runs/fonts excluded, sibling order via id_sort_key — all tested; no duplicated traversal elsewhere in the repo.
- verify integration fixtures: invalid_sampler covers tree/ids/references/palettes/enums families end-to-end via `run()`; apply_test covers text-runs and multi-node routing.
- scripts/add_license_headers.sh: `set -euo pipefail`, prunes target/dist/.git, idempotent apply, --check mode wired into CI.
- .github/workflows: both minimal and correct; push-branch constraint documented to avoid duplicate runs (only the §2-vs-§4 comment pointer, F6).
- Trunk.toml port choice documented; run.sh parses the port from Trunk.toml (single source of truth) with a fallback; clean child shutdown via trap + `wait -n`.
- rustfmt.toml: both deviations from stock carry rationale.
- config/default_keybinds.json: valid JSON, self-documenting (only the `cancel_mode` staleness, F17).
- loader save path: deterministic (BTreeMap ordering) + atomic, both pinned by tests; skip_serializing_if symmetry pinned by double-roundtrip tests.
- README.md: honest about status and scope; quickstart commands match the scripts (modulo F21's convert-verb subset).
