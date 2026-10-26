# Test Conventions

## §T1 Test fundamentals first, and test them heavily

Fundamentals get the heaviest coverage in the repository. These are
the surfaces every other piece of code rests on, and a regression in
one of them ripples into everything above it.

The fundamentals are:

- **Mutations and undo round-trips** — every `GfxMutator` variant,
  every `UndoAction` variant, every forward-and-back. A mutation that
  cannot be undone correctly is a corruption waiting to happen.
- **Unicode and grapheme handling** — every primitive in
  `lib/baumhard/src/util/grapheme_chad.rs`. Emoji, combining marks,
  regional indicators, ZWJ sequences. Test the surprising inputs
  before the obvious ones.
- **Geometry and region indexing** — `almost_equal` /
  `almost_equal_vec2`, `RegionIndexer`, the region params math.
  Every spatial-index assertion is a frame that does not stutter under
  selection.
- **Loader edges** — every shape `.mindmap.json` can take, including
  malformed ones, missing fields, and unknown `edge_type` values.
- **Platform-shared logic** — gesture math, viewport math, anything
  that has to behave identically on native and WASM.

Features built on top of the fundamentals get coverage proportional to
their user impact, but **never less than the happy path plus each
distinct error path**. When in doubt, write the test (§T12).

## §T2 Where tests live

Two patterns coexist, and both are intentional.

### §T2.1 Inline `#[cfg(test)] mod tests` — the default

Most test modules live inline at the bottom of the source file they
test, wrapped in `#[cfg(test)] mod tests { ... }`. This is the default
pattern for anything that does not need to be called from outside the
test harness.

Representative exemplars:
- `src/application/document.rs` — fundamentals coverage of
  hit-testing, selection, undo stacks, portal mutations.
- `lib/baumhard/src/mindmap/border.rs` — border run-spec geometry
  and the resolver cascade.
- `lib/baumhard/src/mindmap/connection.rs` — anchor points, path
  sampling, Bezier curves.

If you are not sure where a new test should go, put it inline.

### §T2.2 `pub mod tests;` trees — Baumhard-only, benchmark-reusable

In `lib/baumhard`, certain modules expose tests through a dedicated
`tests` subdirectory declared as `pub mod tests;` rather than
`#[cfg(test)] mod tests`. This is deliberate and load-bearing: the
benchmark harness at `lib/baumhard/benches/test_bench.rs` imports the
test bodies so criterion can run them as micro-benchmarks.

If the module were gated with `#[cfg(test)]`, the benchmark binary
(which is not compiled under `cfg(test)`) could not reach it. So we
make the test module `pub` and rely on the `do_*()` / `test_*()`
naming split (see §T3) to keep `#[test]`-annotated wrapper functions
out of the benchmark import path.

Do not "fix" the missing `#[cfg(test)]`. It is the way.

Exemplars:
- `lib/baumhard/src/util/tests/` — geometry, color, grapheme, arena,
  primes (file is `primes_test.rs`; the singular suffix is a known
  inconsistency).
- `lib/baumhard/src/gfx_structs/tests/` — region, tree, model, area,
  walker.
- `lib/baumhard/src/core/tests/` — primitives (ranges, color regions).

### §T2.3 Cross-crate rule

The `pub mod tests;` pattern is Baumhard-only. The `mandala` crate has
no benchmark harness, so there is no reason to ever reach for it
there — every mandala-side test lives inline.

Both manifests now say this rather than only this document: `mandala`
carries no `criterion` dev-dependency and both crates set
`autobenches = false`, so baumhard's single declared `[[bench]]`
target is the workspace's only one. That closes a gap this section
used to describe inaccurately — each crate had a `benches/_template.rs`
which cargo auto-discovered and built against the *default* libtest
harness, where `criterion_main!` is ignored: the binaries ran zero
benchmarks and said nothing about it. A new benchmark has to be
declared explicitly with `harness = false` to exist at all.

## §T3 Naming

- **Test functions:** `test_<topic>_<specific_case>`. Lowercase
  snake_case. Examples: `test_hit_test_direct_hit`,
  `test_portal_endpoint_color_channels_are_independent`.

  **Every test named in this document, and in any comment or spec in
  the repository, is a real one.** `baumhard`'s
  `test_no_doc_cites_a_test_that_does_not_exist` scans every Markdown
  file and every `//` comment in the workspace for `test_*`
  identifiers and holds each against the set the sources declare, so a
  renamed test cannot leave a citation pointing at nothing. Two
  citations had already rotted that way before the scan existed. Write
  a *shape* rather than a name when you mean a shape — the
  `test_<topic>_<case>` spelling above, or a trailing `*` for a family
  (`test_delete_node_*prefix*`) — and the scan reads it as one. Its
  sibling `test_no_doc_names_an_identifier_that_does_not_exist` holds
  the names that are *not* tests to the same standard inside doc
  comments — a backticked `Type::method`, `module::function` or
  multi-word bare name that nothing declares or writes fails the
  suite — with `(gone)` after the closing backtick as the explicit
  opt-out for prose that names a vanished thing on purpose
  (CODE_CONVENTIONS §8).

  **Where the case is a claim, name the claim.** A test whose subject
  is a property of the repository rather than a code path is allowed
  to spend the name on the sentence it asserts —
  `test_no_dependency_version_is_written_in_two_manifests`,
  `test_the_workspace_keeps_warn_and_error_in_release`. Still
  `test_`-prefixed, still snake_case; the `<topic>_<case>` shape is
  what relaxes. The reason to allow it: the whole value of these tests
  is the sentence in the failure output, and a two-word name for
  either of them would tell a reader nothing they can act on. This is
  a narrow license — for a test over ordinary code, `<topic>_<case>`
  still wins, because the topic is how you find its neighbors.

- **Benchmark-reusable bodies:** `pub fn do_<topic>_<case>()`, with a
  one-line `#[test] fn test_<topic>_<case>()` wrapper that calls it.
  The `do_*` function is `pub` so it is reachable from
  `benches/test_bench.rs`. Exemplar from
  `lib/baumhard/src/util/tests/geometry_tests.rs`:

  ```rust
  #[test]
  fn test_90_deg_rotation() {
      do_90_deg_rotation();
  }

  pub fn do_90_deg_rotation() {
      let point = Vec2::new(1.0, 0.0);
      let pivot = Vec2::new(0.0, 0.0);
      let rotated = clockwise_rotation_around_pivot(point, pivot, 90.0);
      let expected = Vec2::new(0.0, -1.0);
      assert!(almost_equal_vec2(rotated, expected));
  }
  ```

- **Fixture helpers:** free functions inside `mod tests { ... }`,
  named by what they return. See `load_test_doc` / `load_test_tree` /
  `test_map_path` in `src/application/document.rs`.

- **Lazy-static test data:** `TEST_<NOUN>` in SCREAMING_SNAKE. Pattern
  is in `lib/baumhard/src/gfx_structs/tests/region_tests.rs` and
  `lib/baumhard/src/core/tests/primitives_tests.rs`.

## §T4 Fixtures and test data

- `maps/testament.mindmap.json` is the canonical fixture. Prefer
  loading it through the `load_test_doc()` / `load_test_tree()`
  helpers over hand-constructing `MindMap` literals. A test that
  exercises the loader path at the same time as the feature under
  test catches more regressions for free.

- Build fixture paths with `env!("CARGO_MANIFEST_DIR")` so tests work
  regardless of working directory. The pattern lives in
  `src/application/document.rs`'s test module.

- For heavy repeated data (large region tables, primitive truth
  tables), declare it once per test module via `lazy_static!`. See
  `primitives_tests.rs::OVERLAPS_TEST` and `region_tests.rs` for the
  shape.

- New fixture mindmaps go in `maps/` with a `*.mindmap.json` suffix so
  the build-script walker and the CLI loader both handle them
  uniformly.

## §T5 Assertions

- **Plain `assert!` / `assert_eq!` are the house style.** No
  `pretty_assertions`, no `insta`, no snapshot testing. If a diff is
  hard to read, improve the values you are comparing, not the
  assertion macro.

- **Floating-point and glyph-space geometry** use `almost_equal` and
  `almost_equal_vec2` from `lib/baumhard/src/util/geometry.rs`. Pick
  an epsilon that matches the scale of the value under test — the
  helpers default to a reasonable scale-invariant tolerance, but do
  not trust them blindly for very large or very small values.

- **Panics:** prefer `assert!(matches!(result, Err(_)))` or explicit
  `assert_eq!(result.unwrap_err(), ...)` for fallible APIs. Bare
  `#[should_panic]` is acceptable when a constructor enforces an
  invariant by panicking (see `region_tests.rs`'s RegionParams
  prime-dimension tests for the shape). When the panic message itself
  is load-bearing — i.e. the test exists to verify the message — use
  `#[should_panic(expected = "...")]` so a future reword cannot
  silently weaken the test.

- **Every assertion must be able to fail on its subject.** Naming the
  input that turns it red, and having seen it red, is part of writing
  it — §T13 is the checklist and the control rule.

## §T6 Benchmark-reuse discipline

Any `do_*()` function exported through a `pub mod tests;` tree is part
of `lib/baumhard/benches/test_bench.rs`'s surface. Renaming or
removing one is a two-file change — update the benchmark imports in
the same commit. `cargo test` will not tell you: the benchmark file is
not compiled under `cfg(test)`. `./test.sh`'s `cargo check --workspace
--benches` step will, and that is the only automated place it is
caught, since `AGENTS.md` forbids agents from running the benchmarks
themselves.

That covers one direction only — an entry whose body is gone. The
other direction, a body that never got an entry, compiles fine
forever, because the bench file imports the test modules by glob.
`test_every_do_body_has_a_bench_entry` is what holds that side:
writing the `do_*()` / `test_*()` pair *is* putting the body on the
bench surface, exactly one entry per body — the check fails on two
as well as on none — and a test with no benchmark value opts out by
being a plain `#[test]` fn instead. Its sibling
`test_every_bench_entry_id_is_unique` holds the ids apart, because
criterion keys a row's stored history by its id.
[`lib/baumhard/CONVENTIONS.md §B8`](./lib/baumhard/CONVENTIONS.md)
names the four shapes a body cannot take the split in, the one that
looks like it should and does not, and — counted rather than
implied — the plain `#[test]`s that are simply unconverted.

## §T7 When to add a regression test

Any of these triggers a test in the same commit:

- A new mutation or undo variant — especially if it touches tree
  structure or reparenting logic.
- A new loader path or loader edge case.
- A scene-builder math path that decides where a glyph lands.
- A reported bug — write the test first, name it after the symptom,
  then fix the bug.
- Anything you catch yourself re-verifying by hand across sessions.
  That manual check is a regression test begging to be written.
- Any change to a fundamental (§T1). Touching a fundamental without
  adding to its test surface is technical debt, and §5 of
  [`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) does not tolerate
  technical debt.

## §T8 GPU and renderer testing

We do **not** exercise `Renderer::new`, the wgpu device/queue,
cosmic-text rasterization, or any live GPU code path in tests. The
renderer is constructed once at app startup and never in a test
harness. Standing up a headless wgpu instance for tests is a tar pit.

What we *do* test from the renderer is the pure layout math — cull
rects, palette frame sizing, palette scroll windows, sacred-border
layout, backdrop alignment. See `application::renderer::tests` for the
exemplar. If a bug requires a live wgpu device to reproduce, reproduce
it with the app itself, outside the suite, rather than building a
harness to chase it: `mandala --headless --screenshot out.png <map>`
draws a frame with no display (Mesa's lavapipe on a GPU-less box), and
the `mandala-drive` skill drives and screenshots a running instance —
`scripts/ipc_smoke.sh` is that loop as a check. What the suite pins of
the code around the GPU is what can be pinned without one: the
adapter plan (`renderer/adapter.rs`), the loop body's order against a
recording fake (`app/pump.rs`), the headless frame pacing.

## §T9 Mobile and WASM

The cross-platform reality (see
[`CODE_CONVENTIONS.md §4`](./CODE_CONVENTIONS.md) and §3) shapes how
we test for non-native targets.

- **Tests run on native.** `./test.sh` exercises the entire suite
  against the host target. Platform-shared logic that passes on
  native is trusted to pass on WASM, because the logic does not
  depend on the platform.
- **`build.sh` validates the wasm32 cross-compile.** A change that
  builds for native but breaks `wasm32-unknown-unknown` is a
  regression caught at build time, not runtime.
- **Pure platform-shared logic must be reachable without a wgpu
  instance.** Touch-input math, gesture recognition, viewport math,
  hit-test math — anything that has to behave identically on a
  desktop and a phone — lives in functions that take their inputs as
  plain values and return plain values. This is an architectural
  constraint cross-referenced in
  [`CODE_CONVENTIONS.md §3`](./CODE_CONVENTIONS.md): code that only
  works inside a `wgpu::Device` cannot be tested for cross-platform
  correctness, and code that cannot be tested for cross-platform
  correctness will eventually diverge between platforms.
- **No `wasm-bindgen-test`.** Cross-platform logic is tested once on
  native; WASM-specific code paths are validated by `build.sh`
  compiling for `wasm32-unknown-unknown`.
- **One exception, and the reason for it: the font-lock probe.**
  Compiling proves a path type-checks, not that it runs, and
  `std::time::Instant::now()` is the standing example: it compiles for
  `wasm32-unknown-unknown` and traps the first time it executes there.
  `acquire_font_system_write` read a clock before its first attempt, so
  the browser's font initialization trapped with no message while every
  gate was green. `lib/baumhard/tests/wasm_font_lock.rs` therefore
  builds a small `cdylib` that calls the lock helpers and runs it under
  `node`, and asserts both that the uncontended path works and that
  each same-thread re-entrancy case panics with its own message. It is
  deliberately narrow — no browser, no DOM, no GPU — and it is the
  template if another wasm-only runtime hazard needs pinning: keep the
  probe tiny, and judge pass or fail by the panic *message*, because a
  wasm trap carries no cause of its own.
  It needs the wasm32 target and `node`, and **skips with a note when
  either is missing**, so a green local run without them proves nothing
  about the browser — the same caveat as `test.sh`'s wasm32 gate. CI
  sets `MANDALA_REQUIRE_WASM_PROBE`, which turns a missing prerequisite
  into a failure there.

## §T10 What we deliberately don't do

These are not accidental omissions — each is a decision. Do not
re-litigate them without a strong reason.

- **No `pretty_assertions`, no `insta`, no snapshot testing.** Plain
  assertions, always.
- **No `mockall` or hand-rolled trait mocks.** Tests construct real
  objects and real data. The codebase is small enough that this works.
- **No async test harness.** The app is single-threaded (see
  `CLAUDE.md`). Tests stay that way.
- **No `wasm-bindgen-test`.** See §T9.
- **No GPU / live-wgpu test infrastructure.** See §T8.
- **No coverage gate in CI.** There *is* CI —
  `.github/workflows/test.yml` runs `./test.sh --lint` on every push
  to main and every pull request, on the toolchain `rust-toolchain.toml`
  pins (wasm32 target included), and `license-headers.yml` runs the
  SPDX check beside it. CI runs the same script you do, so
  `./test.sh` stays the covenant rather than becoming a formality
  something else performs. `--coverage` is yours to run locally.
  This bullet used to exclude the lint gates too. That held while
  clippy was advisory and the toolchain floated; once clippy was hard
  at a zero baseline, leaving `--lint` out of CI meant a new stable
  release could turn every contributor's run red while CI, which
  never ran clippy, stayed green and could not catch a real
  regression either. The pin and the CI leg landed together for
  that reason.

## §T11 Running the suite

- `./test.sh` — full suite across every workspace member —
  `mandala`, `baumhard`, `mandala_derive`, `maptool`, `mandalactl` — then a test
  count, then the bench-target type-check and the wasm32 type-check
  gate. Read that order literally: the count prints where the
  *suite* ends, not where the run does, so a `== N tests passed ==`
  scrolling past still has two gates to clear. It is a convenience
  either way and cannot fail the run. The wasm32 gate is skipped
  with a note when the target is not installed, so it is
  unconditional in CI (which installs it) and conditional locally.
  It runs `cargo test --workspace` rather than a list of `-p` flags,
  on purpose: the list it used to carry named three of the four
  members and so never ran `mandala_derive`'s tests at all.
- `./test.sh --coverage` — runs under `cargo-llvm-cov` (install with
  `cargo install cargo-llvm-cov`). HTML at
  `target/llvm-cov/html/index.html`, LCOV at
  `target/llvm-cov/lcov.info`.
- `./test.sh --lint` — also runs `cargo fmt --all -- --check`
  (hard: the tree stays formatted, #130), clippy on the host target
  and on wasm32-unknown-unknown (hard at a zero-warning baseline,
  run with `-D warnings`), and two `cargo doc` legs — the whole workspace with
  private items on the host, and mandala on wasm32 — each hard at a
  zero-warning baseline (#134). A hard-gate failure prints a
  `FAILED:` line inline and the run exits non-zero at the end, after
  every gate has reported.
- `./test.sh --bench` — also runs the benchmarks after tests pass, by
  calling `./bench.sh` rather than repeating its invocation.
  Maintainers only: `AGENTS.md` forbids automated agents this flag,
  `./bench.sh`, and `cargo bench` alike, along with any performance
  claim lacking the main-against-main control row
  [`lib/baumhard/CONVENTIONS.md §B7`](./lib/baumhard/CONVENTIONS.md)
  requires — §B7 is the sole source of that rule; §T6 above governs
  the `do_*()` surface and says nothing about control rows.
  Proving a bench target still *compiles* needs no flag —
  `./test.sh` type-checks all of them on every run, which is what
  keeps §B8's `do_*()` contract enforceable now that its two named
  mechanisms are off limits.
- `cargo test -p baumhard --lib <pattern>`,
  `cargo test -p mandala <pattern>`,
  `cargo test -p mandala_derive`, `cargo test -p maptool` or
  `cargo test -p mandalactl` — targeted subset while iterating.
  `--lib` selects a library target, so it belongs only to the three
  members that have one, baumhard, mandala_derive and mandalactl;
  `mandala` and `maptool` are binary crates, where
  adding it narrows nothing and exits 101 with `no library targets
  found in package 'mandala'` before a test runs. Both this document
  and `CLAUDE.md` put the flag on the mandala line anyway, copied off
  the baumhard neighbor where it is correct (#148);
  `test_no_documented_command_targets_a_lib_the_package_lacks` now
  reads both sections and holds every `-p` and every `--lib` in them
  against the workspace's real manifests.
- `cargo doc -p baumhard --no-deps` — render the library docs and
  spot-check that every `pub` item has the doc comment
  [`lib/baumhard/CONVENTIONS.md §B9`](./lib/baumhard/CONVENTIONS.md)
  requires.

## §T12 Test aggressively

When in doubt, write the test. An untested fundamental is technical
debt (§T1, §T7), and a featureful path with no error-case coverage is
half a feature.

Aggressive is not the same as numerous. A test that cannot fail on
its subject (§T13) adds one to the count and nothing to the coverage,
and it is worse than the gap it hides, because the gap no longer
looks like one.

## §T13 Assertions that cannot fail

A test that cannot fail for the reason it claims to exist is worse
than no test: it passes CI, reads as coverage, and stops the next
person from writing the real one. The defect is invisible by
construction — a vacuous assertion looks exactly like a satisfied
one — so no amount of re-reading the green output finds it. Across
the six PRs of the July 2026 review epic (#54: #126, #127, #128,
#131, #132, #133) it was the most common real defect found, ahead of
wrong code, and four of the six instances below were found by a
reviewer rather than by the author. #138 is the record for that count
and for the instances whose defective form is no longer in the tree.
Two rules carry most of this section:

> **Before committing an assertion, name the input that makes it
> fail.** If you cannot, it is not a test.
>
> **An assertion whose expected value is computed by the code under
> test is a mirror.** Compute it independently, or assert a literal.

### §T13.1 The six shapes

Check every new or changed assertion against each of these. Each
names the instance that taught it.

1. **No true positive available.** A clause whose only reachable
   inputs are correct ones. A line-model test in
   `lib/baumhard/src/util/tests/grapheme_chad_tests.rs` asserted
   `i + 1 == count || !line.ends_with('\r')`, aimed at a line that
   swallowed its own terminator. Under the test's own cluster-index
   reconstruction the terminator's CR always belongs to the cluster
   *after* the line's range, so no input could make the clause fire on
   broken code — while it *would* have fired on correct code the day a
   `\r\r\n` fixture was added. The clause was later shown to be
   redundant outright: the test's other properties already determine
   the whole range table uniquely, so no predicate over that table
   could have added coverage. `test_line_model_is_coherent` is the
   fixed shape: instead of a predicate over its own reconstruction,
   it holds the byte-level `slice_to_newline` — the member of the
   family that *can* swallow a terminator — against the cluster
   bounds. The question that finds this shape is *what does the
   broken output look like, and can the test produce it?*

2. **Default-shadowed.** The value under test also arrives from a
   fallback. Resolving the shipped keybinds template and asserting
   Escape → `ExitMode` passed with the template's key renamed, and
   would have passed with the template reduced to `{}`, because the
   built-in `exit_mode` default is Escape too.
   `test_shipped_keybinds_template_escape_exits_the_mode` is the fixed
   shape: it first moves the file's binding to a combo no default
   carries and demands the move be visible, which proves the value
   comes from the file, and only then asserts the shipped binding.
   Whenever a value has a default, a fallback, an inherited tier or a
   cascade behind it, the fixture must make the tier under test differ
   from every tier behind it.

3. **Only a universal prefix.** An error assertion satisfied by every
   failure. `err.contains("parse keybinds JSON")` matches the prefix
   `KeybindConfig::from_json` puts on a bare `{`, on a wrong value
   type, and on text that is not JSON at all, so it would still pass
   had the input started failing for an unrelated reason —
   `clear_zoom` deleted from the table, say.
   `test_clear_zoom_rejects_the_parametric_binding_shape` keeps the
   prefix and adds the discriminating half — serde naming the value
   shape it refused. Assert the part of the message only *this*
   failure produces.

4. **Captured but never asserted.** A field rendered into a fixture,
   signature or comparison that no row ever exercises. The console
   oracle (`src/application/console/tests/oracle.rs`) rendered
   `font_family` into each candidate's signature, but no corpus row
   produced a candidate carrying one: a mutant setting it to a
   sentinel left every oracle test green, and so did checking it with
   `is_some()` alone.
   `test_console_oracle_font_vocabularies_track_the_loaded_families`
   is the fixed shape: every family row must carry *its own* family,
   not merely some family. Rendering a field is a promise to check it;
   a row that makes it non-default, asserted against the value it
   should hold, is what keeps the promise.

5. **Two tables at different depths.** When one behavior is pinned
   from two sides — a corpus of lines that execute and a corpus of
   lines that complete, say — the shallower table's silence hides the
   gap. `EXEC_CORPUS` pinned `canvas border side top reset` as working
   while `COMPLETION_CORPUS` had no matching row, so the path looked
   pinned and its completion side was never examined. Paired tables
   are compared row against row, or by a test that holds one to the
   other; their row counts are not evidence of anything.

6. **Two derivations sharing a source — the mirror.** `serde_coverage`
   published a list of field spellings and checked it with a walker,
   but both came from the same token matching, so the doc and the
   walker could agree on being wrong and nothing was left to
   disagree. `test_the_scan_and_the_derived_impls_agree_on_a_planted_shape`
   is the fixed shape: the scan is held against what `serde_derive`
   actually accepts, a source that shares no code with it. The same
   epic hit the shape twice at smaller scale — a plant macro whose two
   halves were described as generated from one list while the field
   list was duplicated as a literal, and a test computing its expected
   width through the same metric-cache path as the code under test.
   `test_measure_max_glyph_advance_takes_the_widest_and_falls_back_on_nothing`
   is the fixed shape of the second: its expectations come from
   shaping each cluster in a scratch `cosmic_text::Buffer`, not from
   the `glyph_advance` the subject calls, so an algorithm that agreed
   with itself no longer passes by doing so. That is the *compute it
   independently* half of the second rule above, which is this shape
   stated as a requirement.

### §T13.2 The negative control is a requirement

**Every assertion reported as passing has been seen failing**: break
the thing it guards, watch it go red for the reason its message
gives, revert. That holds for every assertion, not a sample of
them. The pull request that adds or changes the test then says what
was broken and what went red, for every assertion whose failing input
is not self-evident from its own text — a reviewer cannot rerun a
control they were never told about. The control is run, not
committed; what is committed is the test it vouches for.

Two refinements, each paid for in that epic:

- **The control must disable the mechanism on the path the test
  exercises.** A panic-safety test was "controlled" with
  `std::mem::forget` placed after the body. On the unwind path that
  line is never reached, so the guard still ran, the test stayed
  green, and the control read as proof that the test was vacuous — a
  false negative about the test rather than a false positive about
  the code. The correct control restored the pre-fix shape. A control
  that does not go red has proved something only once you know it
  actually broke what it meant to break.
- **A control that goes red for the wrong reason has not controlled
  anything.** If the failure message is not the one the assertion
  carries, the test failed on a precondition, a fixture load or a
  neighboring assertion, and the one under review is still unseen.

### §T13.3 Precondition assertions are not ceremony

A test whose meaning depends on its fixture holding some property
asserts that property before the assertion that depends on it, with
a message that says what the test would otherwise prove.
`test_border_show_reports_the_effective_frame_color` guards its
readout assertion with `assert_ne!(effective, node.style.frame_color,
"the fixture must keep its tiers apart or this test proves nothing")`,
and
`test_the_console_source_scan_reads_what_it_claims_to` exists because
a scan that read nothing finds nothing. These guards are the cheap,
committed half of §T13.2: they turn shape 2 and shape 4 from a silent
pass into a red run the day the fixture drifts. One of them caught a
wrong premise in its own test's first draft — the author had assumed
an empty override left no themed tier, when it falls through to the
group's. Do not delete one as redundant because it has never fired;
that is what it looks like while it is working.

### §T13.4 The same defect one level up

A gate that cannot fail is this defect applied to the suite instead
of to a test. #130 and #134 are its siblings — #134 an instance: a red
doc gate that printed one line and exited 0 for a full review round.
The same two rules apply to anything in `test.sh` or CI: name the
input that turns it red, and make sure a red result changes the exit
code.
