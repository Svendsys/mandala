# P2-40: Workspace hygiene — no [workspace.dependencies], strum at two versions, dead deps in every manifest, undefined release-lto profile breaks `build.sh --fat`, mandala_derive tests never run

**Severity:** P2 (build correctness + compile-time waste + SSOT) · **Area:** workspace manifests + scripts · **All items verified empirically**

## Problems

1. **`./build.sh --fat` is hard-broken**: it runs `cargo build --profile release-lto`, but no `[profile.release-lto]` exists in the root Cargo.toml → `error: profile 'release-lto' is not defined` (reproduced). CLAUDE.md documents `--fat` as supported. build.sh also `rm -rf`s output dirs (including `dist/`) *before* cargo runs, so the failing path still deletes prior artifacts.
2. **No `[workspace.dependencies]`**: ~16 dep versions duplicated verbatim across the three manifests; already diverged — `strum 0.27` (mandala) vs `0.28.0` (baumhard) compile **both** versions plus two `strum_macros` proc-macro builds (`cargo tree -d` confirms).
3. **Dead dependencies** (grep-verified zero code usage):
   - mandala: `syn` (one of the heaviest compile deps, as a *runtime* dep of the app), `serde-lexpr`, `futures` (wasm uses `wasm_bindgen_futures`, native `pollster`), `cosmic-text` (direct — only doc comments mention it; removing it makes the §B5 containment **compiler-enforced**), `tinyvec`, `ttf-parser`; plus a whole `[build-dependencies]` block (walkdir/path-slash/ttf-parser/regex/lazy_static) with **no build.rs** in the root crate (copy-paste of baumhard's, which does have one).
   - baumhard: `syn`, `serde-lexpr`, `smol_str`, `enumset`; `crossbeam-channel`'s only purpose is the unused `_scene_index_sender` parameter (see P2-43); `env_logger` sits in a library crate solely for `util::log::init` (fine to keep if that init API is deliberate — document; otherwise feature-gate); `rand` is pulled into the production lib by one "test-only" pub helper (`get_some_font` — see P2-42).
   - After P2-38: mandala's `unicode-segmentation` also becomes removable.
4. **`lib/mandala_derive`'s 13 tests never run**: `test.sh:67` runs `cargo test -p baumhard -p mandala -p maptool` — the fourth workspace member is excluded (and TEST_CONVENTIONS §T11 says "both crates", stale twice over).
5. **mandala bench template can't work**: `benches/_template.rs` is auto-discovered and compiled under the default libtest harness (no `[[bench]] harness = false` in the root manifest), which ignores `criterion_main!` — silently 0 benchmarks; TEST_CONVENTIONS §T2.3 says the mandala crate has no benchmark harness at all. baumhard's `benches/_template.rs` equally compiles as a stray bench executable on every `cargo bench`.

## Fix plan

1. Add `[profile.release-lto] inherits = "release"` + `lto = "fat"` + `codegen-units = 1` to the root manifest; make build.sh clean outputs only after a successful build (or preflight the profile).
2. Introduce `[workspace.dependencies]`; migrate all shared deps to `dep.workspace = true`; unify strum on 0.28.
3. Delete every dead dep listed (verify each with `cargo build` + `./test.sh` + wasm check; `cargo machete`/`cargo udeps` as a cross-check).
4. `test.sh`: `cargo test --workspace` (or add `-p mandala_derive`); update §T11 wording.
5. Delete both `_template.rs` benches + mandala's criterion dev-dep (aligning with §T2.3), or wire them properly with manifest sections — pick one, delete the half-state.

## Acceptance criteria

- `./build.sh --fat` produces a binary; `cargo tree -d` shows no intra-workspace-caused duplicates.
- `cargo machete` (or equivalent grep audit) clean.
- `./test.sh` count includes mandala_derive's 13 tests.
- WASM build still green.

## Pointers

`Cargo.toml` (root), `lib/baumhard/Cargo.toml`, `crates/maptool/Cargo.toml`, `lib/mandala_derive/Cargo.toml`; `build.sh`, `test.sh`, `bench.sh`; CLAUDE.md (Common tasks); TEST_CONVENTIONS §T2.3/§T11.
