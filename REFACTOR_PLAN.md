# Test-Duplication Exorcism — Refactoring Plan

> *"Repetition of *shape* (near-duplicate code) is a smell; repetition
> of *idiom* (same naming, error posture, lock discipline) is the
> point. Honour the idioms; unify the shapes."*
> — `CODE_CONVENTIONS.md §2`

This document plans a comprehensive refactoring effort to eliminate
test duplication across the workspace. It is the formal scope, the
phasing, the consolidation rules, and the verification regimen — not
a one-shot recipe. Treat it as the contract for a sustained marathon
of small, focused commits.

The work spans both crates (`mandala/src/` and `lib/baumhard/src/`)
and is sized at roughly **eight to twelve commits**, each
independently buildable and `./test.sh`-green. Per
`CODE_CONVENTIONS.md §12` we land *one conceptual change per commit*.

---

## §0 Why this exists

The user surfaced two specific cases:

- `drive_throttle_over_budget` was duplicated **15 times** before
  being consolidated to
  `src/application/app/throttled_interaction/test_utils.rs:9`.
  Despite that consolidation, **`moving_node.rs:151` still re-defines
  it locally** — proof that the consolidation seam was not driven
  through to every consumer.
- `fixture_edge` was duplicated **11 times**, similarly consolidated.
- `test_should_perform_drain_false_when_throttle_skipping` exists as
  **the literal same test body** in `portal_label.rs` and
  `edge_label.rs` (and structurally identical in `edge_handle.rs`,
  `moving_node.rs`, `color_picker_hover.rs`).

These are not accidents. They are a pattern — a "ghetto pattern" in
the user's words — where each new throttled interaction was authored
by **copying the test scaffold from its sibling** and tweaking the
fixture name. The result is a codebase that looks like five honest
implementations of one trait *and* five copies of the same five
tests of that trait. The trait test belongs **on the trait**, not
on every implementor.

Subagent surveys (mandala-side and baumhard-side) have catalogued
**at least nine distinct duplication categories** beyond the
throttled_interaction case. They are listed in the per-phase sections
below with file:line citations.

---

## §1 Governing principles

These are enforced for every chunk in the plan. Deviating from them
is grounds to revert the chunk and try again.

### §1.1 Shape vs. idiom

`CODE_CONVENTIONS.md §2` is the north star. The shapes — the
literal repeated code blocks — get unified into one home. The
idioms — naming, error posture, fixture discipline — stay
identical *because they were already identical and that's the
point*.

### §1.2 Three-strikes rule modulated by intent

`CODE_CONVENTIONS.md §7`: *Three similar lines beats a premature
abstraction. Extract a helper when a pattern repeats three times
**and** the repetition obscures intent.*

The duplications listed in this plan all clear both bars. Two
occurrences alone do not qualify and are noted but not extracted.

### §1.3 Visibility minimum

A consolidated helper takes the **narrowest visibility** that makes
it reachable from every caller, and no narrower:

- Single module, multiple files in same `mod` tree → `pub(super)`.
- Crate-wide → `pub(crate)`, gated `#[cfg(test)]` at the module level.
- Baumhard-side, benchmark-reachable → `pub` inside a `pub mod tests;`
  tree (per `TEST_CONVENTIONS.md §T2.2`). **Do not "fix" the missing
  `#[cfg(test)]`** — it is load-bearing for benchmark reuse.

Adding `pub(crate)` where `pub(super)` would suffice is a quiet
violation. Reviewers should flag it.

### §1.4 No new top-level test crates, no new test fixtures directory

`CODE_CONVENTIONS.md §6`: *Prefer editing over creating. New files
should feel justified.* The existing homes (`tests_common.rs`,
`test_utils.rs`, `tests/fixtures.rs`, `lib/baumhard/src/util/tests/`)
absorb the consolidation. No `tests-helpers/` crate. No
`testing/` directory.

### §1.5 No mocks, no snapshots, no async harness

`TEST_CONVENTIONS.md §T10` enumerates what we deliberately don't do.
The consolidation never introduces `mockall`, `pretty_assertions`,
`insta`, or any new dev-dependency. Plain `assert!` / `assert_eq!`
remains the house style. If a unified helper is tempted to take a
`Box<dyn Fn>` callback, that's a smell — use a generic over a real
trait instead.

### §1.6 Cross-platform parity preserved

Every consolidated helper that lives under `#![cfg(not(target_arch =
"wasm32"))]` (the entire `throttled_interaction/` tree, for example)
keeps that gate intact. Helpers usable on both targets stay
unconditional. `./test.sh`'s WASM type-check gate (per
`CLAUDE.md`'s "Common tasks") fails the run if a consolidation
accidentally exposes native-only types to a shared helper.

### §1.7 Benchmark sync for baumhard `do_*()` renames

`TEST_CONVENTIONS.md §T6` and
`lib/baumhard/CONVENTIONS.md §B8`: any rename or removal of a
`do_*()` function in baumhard is a **two-file change** —
`lib/baumhard/benches/test_bench.rs` is updated in the same commit.
The compiler does **not** catch drift. `./test.sh --bench` does.

### §1.8 Every chunk leaves the suite green

`CODE_CONVENTIONS.md §5` and `§T11`: `./test.sh` passes between
every commit. A red-then-green sequence is **not** an option. If a
chunk grows past one green-to-green step, split it further.

---

## §2 Naming conventions for the consolidated surface

The new helpers MUST follow the existing house style
(`TEST_CONVENTIONS.md §T3`). No drift.

| Kind                         | Style                              | Example (existing)                         |
|------------------------------|------------------------------------|--------------------------------------------|
| Test fn                      | `test_<topic>_<case>`              | `test_hit_test_direct_hit`                 |
| Bench-reusable body          | `pub fn do_<topic>_<case>()`       | `do_90_deg_rotation`                       |
| Fixture builder (returns T)  | named for what it returns          | `load_test_doc`, `fixture_edge`            |
| Loader cache lazy-static     | `TEST_<NOUN>` SCREAMING_SNAKE      | `TEST_OVERLAPS`                            |
| Macro-based test scaffold    | `<verb>_for_throttled_interaction!`| (new — see §3)                             |

**Macros are a last resort.** Prefer a generic function over a
real trait. Reach for `macro_rules!` only when the helper needs to
**generate `#[test]` attributes** (i.e. the case where one helper
must emit N independent test functions so each shows up in the suite
output by name). The throttled_interaction phase below is the only
case in this plan that meets that bar.

---

## §3 Visibility decisions, file by file

The proposed homes for consolidated helpers (full rationale per
phase):

| Helper                                        | Home                                                                  | Visibility                |
|-----------------------------------------------|-----------------------------------------------------------------------|---------------------------|
| `drive_throttle_over_budget`, `fixture_edge`  | `src/application/app/throttled_interaction/test_utils.rs` (existing)  | `pub(super)` (unchanged)  |
| Drain-trait test macro                        | same file, new                                                        | `macro_rules!` (no `pub`) |
| `load_test_doc`, `test_map_path`              | `src/application/document/tests_common.rs` (existing)                 | `pub(crate)` (existing)   |
| `first_node_id(doc)`                          | `tests_common.rs` (existing — replaces hardcoded "0")                 | `pub(crate)`              |
| `make_test_mutation` family                   | `tests_common.rs` (new section)                                       | `pub(crate)`              |
| Console `run()` helper                        | `src/application/console/tests/fixtures.rs` (existing canonical)      | `pub(super)` (existing)   |
| `sample_geometry` (color picker)              | re-export from one fixtures.rs into the other                         | `pub(super)` re-export    |
| Baumhard-side helpers                         | per-module `tests/` directories per existing `pub mod tests;` pattern | `pub` (existing)          |

No file is created unless an existing one would grow past its
concept boundary. In practice, this plan creates **zero new files**;
every consolidation lands in an existing `test_utils.rs` /
`tests_common.rs` / `fixtures.rs`.
---

## §4 Phase A — `throttled_interaction/` exorcism (commits 1–3)

This is the most egregious duplication site, and the one the user
explicitly named. Five files implement the same trait
(`ThrottledInteraction`) and each carries five copies of the same
trait-level test, with only the fixture and the "set pending" line
varying.

### §4.1 Inventory of duplication (mandala survey, confirmed)

**Group A — exact-text duplicates of the trait-default tests:**

The following five tests appear with structurally identical bodies
in five files (only the fixture and the pending-state line differ):

- `test_should_perform_drain_false_when_idle`
- `test_should_perform_drain_true_when_pending_and_throttle_fresh`
- `test_should_perform_drain_false_when_throttle_skipping`
- `test_idle_should_perform_drain_does_not_advance_throttle`
- `test_reset_*` (form varies slightly per impl, but all assert
  "throttle resets, pending state survives")

Affected files:

- `src/application/app/throttled_interaction/edge_handle.rs:183-237`
- `src/application/app/throttled_interaction/edge_label.rs:156-194`
- `src/application/app/throttled_interaction/portal_label.rs:168-206`
- `src/application/app/throttled_interaction/moving_node.rs:226-276`
- `src/application/app/throttled_interaction/color_picker_hover.rs:166-205`

**Group B — `drive_throttle_over_budget` re-defined locally:**

- `test_utils.rs:9` — canonical
- `moving_node.rs:151` — byte-identical local copy with similar
  doc-comment. Delete; import the canonical one.

**Group C — fixture-level duplication between sibling files:**

- `test_new_initialises_pending_cursor_to_none` — identical body
  in `edge_label.rs:111` and `portal_label.rs:118`.
- `test_has_pending_*` and `test_latest_cursor_overwrites_previous`
  — identical bodies in those same two files.
- `test_new_initialises_fields_with_zero_deltas` — structurally
  identical in `edge_handle.rs:139` and `moving_node.rs:161` (same
  invariant on the `Vec2::ZERO` accumulator pair).

### §4.2 Consolidation strategy — a `macro_rules!` in `test_utils.rs`

Because the trait's *default-method* tests must each appear as their
own `#[test]` function (so they show up named in the suite output
and any single failure points at the right impl), we cannot just
write one generic helper and call it from each file. We need a
macro that **emits N test functions per impl**.

The shape of the macro (sketch — exact form determined during
implementation):

```rust
// In src/application/app/throttled_interaction/test_utils.rs:
//
// Emits the five trait-default tests for one ThrottledInteraction
// implementor. The caller supplies:
//   $name      — module name (e.g. moving_node)
//   $ty        — the implementor type
//   $build     — expr returning a fresh idle instance
//   $set_pending — closure taking &mut $ty that flips has_pending
//                  to true (each impl knows its own field)
//
// The macro is bench-irrelevant (mandala-side, not baumhard) so a
// `pub(super)` macro_rules at the test_utils.rs level is fine.
macro_rules! trait_default_tests_for_throttled_interaction {
    ($mod:ident, $ty:ty, $build:expr, $set_pending:expr) => {
        mod $mod {
            #[test]
            fn test_should_perform_drain_false_when_idle() { /* ... */ }
            #[test]
            fn test_should_perform_drain_true_when_pending_and_throttle_fresh() { /* ... */ }
            #[test]
            fn test_should_perform_drain_false_when_throttle_skipping() { /* ... */ }
            #[test]
            fn test_idle_should_perform_drain_does_not_advance_throttle() { /* ... */ }
            #[test]
            fn test_default_reset_resets_throttle_only() { /* ... */ }
        }
    };
}
```

A single invocation per implementor lives in that implementor's
own test module:

```rust
// In moving_node.rs's `mod tests`:
trait_default_tests_for_throttled_interaction!(
    drain_defaults,
    MovingNodeInteraction,
    || MovingNodeInteraction::new(vec!["n".into()], false),
    |i: &mut MovingNodeInteraction| { i.pending_delta = Vec2::new(1.0, 0.0); }
);
```

This collapses **~250 lines of duplicated test logic across five
files into one ~80-line macro plus five 6-line invocations**. Each
test still appears named in the test-binary output (with the module
prefix), so a failure in the moving-node impl points at the right
file.

**Per-impl tests stay where they are.** A test like
`test_handle_variant_round_trips_control_point` (edge_handle.rs:225)
is genuinely impl-specific and does not move. The macro covers
**only** the trait-default behaviour.

### §4.3 Why a macro and not a generic function

Three reasons:

1. **Each test must surface as its own `#[test]`.** A generic
   function called from one wrapper test would collapse five
   distinct failure modes into one, making test-output triage
   harder.
2. **The "set pending" hook is per-field.** `MovingNodeInteraction`
   has `pending_delta: Vec2`, `EdgeLabelInteraction` has
   `pending_cursor: Option<Vec2>`, `ColorPickerHoverInteraction`
   has `dirty: bool`. A trait method to expose this just for the
   tests would leak test-only surface onto production code, which
   `CODE_CONVENTIONS.md §10` actively forbids.
3. **The trait object boxing isn't worth it.** Boxing a generic
   helper into `&mut dyn ThrottledInteraction` for the test loop
   is a noise-introducing layer for no benefit; macros expand at
   the call site against the concrete type.

The macro genuinely earns its keep. If a future reviewer wants to
re-litigate this, the alternative is documented: **expose pending
flips through a test-only inherent helper on each impl, then call
a generic test fn**. That alternative wasn't picked because the
test-only inherent surface is just another duplication shape (one
helper per impl) and the macro avoids it.

### §4.4 Commit boundaries inside Phase A

This phase is **three commits**, each green on `./test.sh`:

- **Commit A1** — *delete the local `drive_throttle_over_budget`
  in `moving_node.rs`*; switch its `mod tests` to import from
  `test_utils.rs`. **No behaviour change**, no test count change,
  just the obvious oversight closed. Smallest possible commit;
  proves the consolidation seam works end-to-end.
- **Commit A2** — *land the
  `trait_default_tests_for_throttled_interaction!` macro* in
  `test_utils.rs`, and apply it to **two** implementors (e.g.
  `moving_node` and `edge_handle`). Pick the two whose test bodies
  the macro must accommodate (delta accumulator vs. snapshot-edge),
  so the macro's parameterisation is exercised on day one.
  Delete the now-redundant tests in those two files; verify the
  test count *stays the same* (the same tests still run, named
  the same, just emitted by the macro).
- **Commit A3** — *apply the macro to the remaining three*
  (`edge_label`, `portal_label`, `color_picker_hover`). Same
  before-and-after test count.

Test count is asserted by the `./test.sh` summary printout.
Acceptable drift: zero.

### §4.5 What the cleanup does NOT touch

- The trait itself (`ThrottledInteraction` in `mod.rs`) is not
  edited. Its tests in `mod.rs:191–305` are *trait-level* tests
  about the dispatcher — distinct concept from impl-level
  trait-default tests, and they stay.
- The per-impl-specific tests stay in their files. The macro is
  for trait-default behaviour only.
- `color_picker_hover.rs`'s `test_canvas_needs_rebuild_*` family
  is genuinely impl-specific (it tests a private predicate not on
  the trait) and stays in place.

### §4.6 Acceptance criteria for Phase A

- `./test.sh` green; total test count delta is **0** (renamed but
  not added or removed).
- `grep -rn "drive_throttle_over_budget" src/application/app/throttled_interaction/`
  shows **one** definition (`test_utils.rs`) and N call sites.
- `grep -rn "fn test_should_perform_drain_false_when_throttle_skipping"
  src/` returns **zero** matches in `*.rs` source — the test name
  now exists only as a macro-emitted identifier inside one or more
  invocations.
- `./test.sh --lint` advisory output reviewed: no new
  clippy warnings.
- WASM type-check gate green (`./test.sh` runs it; the throttled
  module is `cfg(not(target_arch="wasm32"))` so this is a no-op
  in practice, but the gate must still pass).
---

## §5 Phase B — Testament-loader unification (commit 4)

The `tests_common::load_test_doc()` cache exists for a reason — see
`tests_common.rs:5–18` doc comment: *"avoids the `FONT_SYSTEM`
write-lock contention `MindMapDocument::load` would otherwise
trigger N times in a parallel test run"*. Six sites that load the
testament map **bypass that cache** today. They are bugs by
construction (slower than necessary, locking pattern divergent from
the rest), and `CODE_CONVENTIONS.md §5` says we close them now.

### §5.1 Inventory of duplication

**Divergent-shape duplicates of `load_test_doc`:**

- `src/application/console/tests/fixtures.rs:21` — hand-builds
  `MindMapDocument` field-by-field, calls `loader::load_from_file`
  directly (no cache), no `finalize`. Out-of-sync with the
  canonical loader at `tests_common.rs:55`.

**Identical `test_map_path()` copies:**

- `src/application/document/tests_common.rs:29` — canonical
  `pub(crate)`.
- `src/application/console/tests/fixtures.rs:15` — duplicate.
- `src/application/app/text_edit/editor.rs:486` — private copy
  inside `#[cfg(test)]`.

**Inline `format!("{}/maps/testament.mindmap.json",
env!("CARGO_MANIFEST_DIR"))` + `MindMapDocument::load`:**

- `src/application/document/mutations/flower_layout.rs:104` and `:185`
  (two test helpers, both bypassing the cache).
- `src/application/document/mutations/tree_cascade.rs:128`, `:191`,
  `:218` (three helpers, all bypassing).
- `src/application/console/traits/tests.rs:111` — `fresh_doc()`,
  uses `MindMapDocument::load` directly.

**Identical `first_node_id()` helpers:**

- `src/application/document/nodes.rs:837` — walks
  `nodes.keys().next()`.
- `src/application/console/commands/border/tests.rs:27` — same body.

(The canonical `tests_common::first_testament_node_id` at
`tests_common.rs:69` *ignores its `doc` argument and hardcodes
"0"* — a clear sign the helper has bit-rotted under the
duplication. We fix it as part of the same commit so callers don't
have to choose which one to trust.)

### §5.2 Consolidation strategy

One canonical home (`document/tests_common.rs`), already in place,
already used by ~10 sites, just not enforced everywhere. The work
is **deletion** plus **import**:

1. **Delete** `console/tests/fixtures.rs:test_map_path`,
   `console/tests/fixtures.rs:load_test_doc`, and the matching
   `text_edit/editor.rs:test_map_path` private copy. Replace each
   call with `crate::application::document::tests_common::load_test_doc()`
   (or `test_map_path()` as appropriate).
2. **Repoint** the five inline-load sites in `flower_layout.rs`,
   `tree_cascade.rs`, and `console/traits/tests.rs` to
   `tests_common::load_test_doc()`. The helper functions
   (`test_doc_with_children`, `fresh_doc`, etc.) keep their names
   and signatures but their bodies become a one-liner that delegates.
3. **Replace** the divergent `console/tests/fixtures.rs:load_test_doc`
   *body* with a delegate to `tests_common::load_test_doc()`. The
   diverged version skipped `build_mutation_registry()`, so the
   delegate must perform that step explicitly to preserve the
   console tests' expectations. Verify by running
   `cargo test -p mandala --lib console` before and after.
4. **Replace `first_node_id` duplicates with one helper** at
   `tests_common.rs`. Fix the bit-rotted
   `first_testament_node_id(doc)` to **actually** consult the
   doc:
   ```rust
   pub(crate) fn first_node_id(doc: &MindMapDocument) -> String {
       doc.mindmap.nodes.keys().next()
           .cloned()
           .expect("testament map has nodes")
   }
   ```
   Drop the misleading `_doc: &MindMapDocument` parameter on the
   old helper; rename to `first_node_id` to match what the body
   does. Replace both inline copies with the canonical one.

### §5.3 Visibility considerations

The console-side helpers used to be `pub(super)`. Re-exporting
through `tests_common` requires:

- `tests_common.rs::load_test_doc` is already `pub(crate)`.
- `tests_common.rs::first_node_id` (new) is `pub(crate)` to reach
  the `console::commands::*` callers.
- `tests_common.rs::test_map_path` is already `pub(crate)`.

No further visibility widening needed.

### §5.4 Why this is one commit (not many)

The whole phase is structurally trivial — every change is a
delete-and-import — so it lands as one commit. The chunk-counting
discipline (`CODE_CONVENTIONS.md §12`) is *one conceptual change
per commit*; "make every site use the canonical loader" is one
concept. If a single delete-and-import path turns out to need a
behavioural fix (e.g. a test that *depended* on the divergent
no-finalize shape), split that fix into its own commit ahead of
this one.

### §5.5 Acceptance criteria for Phase B

- `grep -rn 'env!("CARGO_MANIFEST_DIR").*testament.mindmap.json' src/`
  returns **one** match: `tests_common.rs:test_map_path`.
- `grep -rn "fn test_map_path" src/` returns **one** match.
- `grep -rn "fn load_test_doc" src/` returns **one** match.
- `grep -rn "MindMapDocument::load" src/` shows **only** production
  call sites (`run_native.rs`, `run_wasm.rs`, the document module
  itself), not test helpers.
- `./test.sh` green; total test count unchanged.
- The `console::tests` suite still passes — important sanity check
  given the divergent-shape consolidation.

---

## §6 Phase C — Fixture-builders (`make_test_mutation` family + `MindNode`/`MindEdge` literals) (commit 5)

### §6.1 Inventory of duplication

**Mutation-builder family — three near-identical factories:**

- `src/application/document/tests_mutations.rs:23`
  `make_test_mutation(id, scope) -> CM` — `NudgeRight(10.0)`,
  no timing, no contexts.
- `src/application/console/commands/mutation.rs:399`
  `make_cm(id, contexts, description) -> CM` — `NudgeRight(1.0)`
  (different magnitude!), no timing, includes contexts/description.
- `src/application/document/tests_hit_move.rs:464`
  `make_test_mutation_with_timing(id, scope, timing) -> CM` —
  `NudgeRight(10.0)` + optional timing.

These are the same factory — only their parameter flexibility
differs. The `NudgeRight(1.0)` vs `NudgeRight(10.0)` discrepancy
is itself a small bug (or, at best, a coincidence); the
consolidated builder makes the choice explicit.

**`make_animated_mutation` at `tests_mutations.rs:365`** is a
specialisation of `make_test_mutation_with_timing` where `timing`
is `Some(AnimationTiming { duration_ms, delay_ms: 0,
easing: Linear, then: None })`. It folds into the base builder.

**`MindNode` / `MindEdge` struct-literal duplication:**

- `src/application/app/tests.rs:184` — `fixture_node(id, x, y) ->
  MindNode` with hardcoded `#141414` / `#30b082` / `LiberationSans`.
- `src/application/document/tests_edges_chain.rs:74` —
  `synthetic_single_node_map(text, w, h)` — same 18 fields, same
  values.

`src/application/document/defaults.rs:38` already provides
`default_orphan_node()` with **identical defaults**. Neither test
helper calls it. Both should.

Similarly:
- `app/tests.rs:fixture_edge(false)` (cross_link variant)
  duplicates `defaults::default_cross_link_edge`.
- `throttled_interaction/test_utils.rs:fixture_edge()` already
  exists as a shared parent_child fixture and is correctly
  consumed.

### §6.2 Consolidation strategy

**Mutation-builder consolidation:**

Land one canonical builder in `tests_common.rs` with optional
fields. Sketch:

```rust
// In src/application/document/tests_common.rs
pub(crate) struct TestMutationBuilder {
    id: String,
    scope: TS,
    nudge_amount: f32,
    contexts: Vec<PC>,
    description: String,
    timing: Option<AnimationTiming>,
}

impl TestMutationBuilder {
    pub(crate) fn new(id: &str, scope: TS) -> Self { /* defaults */ }
    pub(crate) fn nudge(mut self, amount: f32) -> Self { /* ... */ }
    pub(crate) fn timing(mut self, t: AnimationTiming) -> Self { /* ... */ }
    pub(crate) fn contexts(mut self, c: Vec<PC>) -> Self { /* ... */ }
    pub(crate) fn description(mut self, d: &str) -> Self { /* ... */ }
    pub(crate) fn build(self) -> CM { /* assemble */ }
}

// Convenience: most callers just want the trivial form.
pub(crate) fn make_test_mutation(id: &str, scope: TS) -> CM {
    TestMutationBuilder::new(id, scope).build()
}
```

The builder is a `pub(crate)` struct (no Default — every test that
constructs one declares its `id` and `scope` explicitly, which is
the contract).

Three call-site rewrites:

- `tests_mutations.rs::make_test_mutation` → delete; use
  canonical.
- `tests_mutations.rs::make_animated_mutation` → delete; rewrite
  callers to `TestMutationBuilder::new(...).timing(...).build()`.
- `mutation.rs::make_cm` → delete; rewrite the **two** callers
  to use the builder. Note the magnitude discrepancy — investigate
  before consolidating; if `NudgeRight(1.0)` was load-bearing for
  any test, we keep it via the builder's `nudge(...)` knob and
  document the choice in the test.

**Node / edge fixture consolidation:**

`defaults::default_orphan_node` is currently `pub(super)` to the
`document` module. Widening to `pub(crate)` lets `app/tests.rs`
reach it directly:

- `app/tests.rs::fixture_node` → delete; rewrite callers to
  `default_orphan_node(id, Vec2::new(x as f32, y as f32))`.
- `app/tests.rs::fixture_edge(portal)` → delete; rewrite callers
  to `default_cross_link_edge(...)` or `default_portal_edge(...)`
  as appropriate.
- `tests_edges_chain.rs::synthetic_single_node_map` → keep the
  function (it builds a `MindMap`, not a node), but rebuild its
  body to call `default_orphan_node` instead of inlining the 18
  fields.

**Visibility widening for `defaults::default_*`:**

The functions are currently `pub(super)`. They need to reach
`app/tests.rs`. Two options:

1. Widen to `pub(crate)` outright. Cleanest, makes the existing
   shape directly reusable by every test. Production callers are
   unaffected; the API surface only grows for in-crate consumers.
2. Add `pub(crate)` re-exports from `tests_common.rs`. More
   indirection.

Choose **option 1**. `defaults` is already a small module of
test-friendly factory functions; `pub(crate)` matches the testament
loader's visibility and avoids the indirection.

### §6.3 What stays inline

- `make_set_bg_doc_mutation` at `tests_mutations.rs:44` — its
  `DocumentAction::SetThemeVariables` payload is genuinely
  specific to the document-actions undo test. Stays.
- `app/tests.rs::FROM_ID`, `TO_ID`, `EDGE_TYPE` constants — local
  to the drag-helper tests, two-line repeat that doesn't clear
  the three-strikes bar.
- `tests_edges_chain.rs::synthetic_single_node_map` (renamed if
  the consolidation produces a clearer name) stays as the
  single-node fixture builder — it returns a `MindMap`, not a
  `MindNode`.

### §6.4 Acceptance criteria for Phase C

- `grep -rn "fn make_test_mutation\b\|fn make_cm\b\|fn make_test_mutation_with_timing\b\|fn make_animated_mutation\b" src/`
  returns **one** match (the canonical convenience function in
  `tests_common.rs`).
- `grep -rn "fn fixture_node\b\|fn fixture_edge\b" src/` returns
  fixtures only in their canonical homes (`throttled_interaction/test_utils.rs`,
  if `fixture_edge` is kept there as a re-export shim, or the
  `defaults` module otherwise).
- `app/tests.rs` no longer contains a 40-line struct literal for
  `MindNode`.
- `./test.sh` green; total test count unchanged.
- `./build.sh --wasm` green (since `defaults` is shared code, the
  visibility widening must not break the WASM build).
---

## §7 Phase D — Console `run()` helper + color-picker fixture re-export (commit 6)

### §7.1 Inventory of duplication

**Console-command `run()` helpers — three private duplicates:**

- `src/application/console/commands/font.rs:452` — tokenises and
  dispatches to `execute_font`.
- `src/application/console/commands/border/tests.rs:21` — same
  shape, dispatches to `execute_border`.
- `src/application/console/commands/mutation.rs:418` — same shape,
  dispatches to `execute_mutation`.

The boilerplate
```rust
let toks = tokenize(line);
let mut eff = ConsoleEffects::new(doc);
super::execute_X(&Args::new(&toks[1..]), &mut eff)
```
is structurally identical across all three. Meanwhile,
`console/tests/fixtures.rs:55` already provides a generic `run()`
that dispatches through `parse()` → `(cmd.execute)` and reaches
*every* command without naming any specific one.

**Color-picker fixture duplication:**

- `src/application/color_picker/tests/fixtures.rs:12`
  `sample_geometry()`.
- `src/application/color_picker_overlay/tests/fixtures.rs:18`
  `picker_sample_geometry()` — same `ColorPickerOverlayGeometry`
  struct, byte-for-byte same field values.

### §7.2 Consolidation strategy

**Console `run()` helper:**

The three per-command helpers should be deleted in favour of the
generic `console::tests::fixtures::run`. Cost: the dispatch goes
through one extra `parse()` call. Benefit: one helper, one body,
identical across every command-suite test.

The mechanical change per file:

1. **font.rs** — delete `fn run` at `font.rs:452`. Add `use
   super::super::tests::fixtures::run;` (or
   `use crate::application::console::tests::fixtures::run;`,
   depending on what compiles cleanly given module visibility).
   Verify `cargo test -p mandala --lib console::commands::font`.
2. **border/tests.rs** — same delete + import. Verify the border
   test suite.
3. **mutation.rs** — same delete + import. Verify the mutation
   test suite.

If a per-command helper test relies on bypassing the generic
parser (e.g. exercising a malformed-input edge that the parser
rejects before `execute_X` would see it), keep that *specific*
test on the per-command path with a helper named for what it
does (`run_raw_tokens`, perhaps), not as a near-duplicate of the
generic `run`. None of the three `run()` helpers surveyed today
appear to need this.

**Color-picker fixture re-export:**

One module owns the canonical `sample_geometry()`; the other
re-exports.

The canonical home is `color_picker/tests/fixtures.rs:12` (the
`color_picker` module is the conceptual owner — the overlay
is a renderer-side companion). `color_picker_overlay/tests/fixtures.rs`
becomes:

```rust
// SPDX-License-Identifier: MPL-2.0
//! Re-export of the shared color-picker geometry fixture.
pub(super) use crate::application::color_picker::tests::fixtures::sample_geometry as picker_sample_geometry;
```

(Or, more cleanly, both call sites import the same name from the
canonical location and the alias goes away. Choose whichever
involves the smaller call-site rewrite — likely the alias, since
the existing call-site name is `picker_sample_geometry`.)

### §7.3 Acceptance criteria for Phase D

- `grep -rn "fn run\b" src/application/console/` returns the
  canonical `console::tests::fixtures::run` and **zero**
  per-command duplicates.
- `grep -rn "fn .*sample_geometry\b" src/application/color_picker*` —
  one canonical definition, optionally one re-export alias.
- `./test.sh` green; total test count unchanged.

---

## §8 Phase E — Baumhard exorcism (commits 7–9)

The baumhard survey turned up four headline duplication categories
and a noisy `fonts::init()` boilerplate situation that needs a
nuanced response. Per `lib/baumhard/CONVENTIONS.md §B0`, the
foundation must be pristine; per `§B8`, every consolidated `do_*`
rename is a two-file change with `benches/test_bench.rs` updated
in the same commit.

### §8.1 Phase E.1 — A shared `mindmap::test_helpers` module (commit 7)

**Inventory:**

- `test_map_path()` identical body in **four** files
  (`mindmap/loader.rs:69`, `mindmap/model/tests.rs:11`,
  `mindmap/tree_builder/tests/fixtures.rs:15`,
  `mindmap/scene_builder/tests/fixtures.rs:14`).
- `synthetic_node` near-identical in
  `mindmap/tree_builder/tests/fixtures.rs:23` and
  `mindmap/scene_builder/tests/fixtures.rs:22`.
- `synthetic_map` structurally identical in
  `tree_builder/tests/fixtures.rs:54` and
  `scene_builder/tests/fixtures.rs:76`.
- `synthetic_portal_edge` identical in
  `tree_builder/tests/fixtures.rs:118` and
  `scene_builder/tests/fixtures.rs:121`.
- `synthetic_edge_with_label` at `model/tests.rs:212` overlaps
  `scene_builder/tests/fixtures::synthetic_edge` shape.

**Strategy:**

Land a new module
`lib/baumhard/src/mindmap/test_helpers.rs`, declared with
`pub mod test_helpers;` in `lib/baumhard/src/mindmap/mod.rs`. The
module follows the Baumhard `pub mod tests;` benchmark-reuse
convention (`TEST_CONVENTIONS.md §T2.2`):

- **No `#[cfg(test)]` gate** — load-bearing for benchmark reuse.
  Items are `pub` so call sites in
  `tree_builder/tests/fixtures.rs`, `scene_builder/tests/fixtures.rs`,
  `model/tests.rs`, and `loader.rs`'s test mod can all reach them.
- Module-level `//!` doc per `§B9` describing the shared-fixture
  concept.
- Each item carries a `///` doc comment per `§B9` (purpose,
  inputs, costs).

Items to land:

```rust
// lib/baumhard/src/mindmap/test_helpers.rs

//! Shared synthetic-map fixtures for tests in the mindmap layer.
//! Co-located with `mindmap/` so tests in tree_builder, scene_builder,
//! model, and loader all reach the same primitives.

use std::path::PathBuf;
use crate::mindmap::model::{MindMap, MindNode, MindEdge, ...};

/// Path to the canonical testament fixture.
pub fn testament_map_path() -> PathBuf { /* ... */ }

/// Build a single synthetic node at (x, y) with optional size +
/// frame visibility. Defaults: 80×40, show_frame=true.
pub fn synthetic_node(id: &str, x: f64, y: f64) -> MindNode { /* ... */ }
pub fn synthetic_node_sized(id: &str, x: f64, y: f64, w: f64, h: f64, show_frame: bool) -> MindNode { /* ... */ }

/// Assemble a `MindMap` from a slice of nodes + a slice of edges.
pub fn synthetic_map(nodes: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap { /* ... */ }

/// Minimal cross-link edge with no anchors, no label.
pub fn synthetic_edge(from: &str, to: &str) -> MindEdge { /* ... */ }

/// Minimal cross-link edge with an explicit label_config.
pub fn synthetic_edge_with_label(from: &str, to: &str, label: &str, config: EdgeLabelConfig) -> MindEdge { /* ... */ }

/// Portal-mode edge with a default ◈ glyph at 16pt.
pub fn synthetic_portal_edge(from: &str, to: &str) -> MindEdge { /* ... */ }

/// Trivial Canvas with `#000` background and empty theme tables.
/// Used by model tests that need a Canvas placeholder but don't
/// exercise canvas behaviour.
pub fn blank_canvas() -> Canvas { /* ... */ }
```

Six call-site rewrites:

- `mindmap/tree_builder/tests/fixtures.rs:23,54,118` — delete
  local `synthetic_*`; re-export from `test_helpers` for the
  `pub(super)` callers if convenient, or rewrite each call site
  to `crate::mindmap::test_helpers::synthetic_node(...)`.
- `mindmap/scene_builder/tests/fixtures.rs:14,22,53,76,121` —
  same delete + replace.
- `mindmap/model/tests.rs:11` — delete local `test_map_path`,
  use canonical.
- `mindmap/model/tests.rs:212` — replace
  `synthetic_edge_with_label` with the canonical one.
- `mindmap/model/tests.rs:309,403,426,495,515,530` — replace
  the six inline `Canvas {}` literals with `blank_canvas()`.
- `mindmap/loader.rs:69` — delete local `test_map_path`, use
  canonical.

**Crucial benchmark-reuse check:** items in `test_helpers` must
be reachable from `lib/baumhard/benches/test_bench.rs` if any
benchmark uses them. The plan is to **not** add benchmark imports
for `test_helpers` initially (no benchmark currently uses
`synthetic_node`); but the `pub mod test_helpers;` shape leaves
the seam open for future benches that want to share the
fixture.

### §8.2 Phase E.2 — `fonts::init()` redundancy audit (commit 8)

`fonts::init()` appears 73 times across baumhard tests. The
function is idempotent (calls a `OnceLock`), so the duplication
is performance-neutral, but it is **conceptual noise**. Two
things to do:

1. **Verify the idempotent contract.** Read
   `lib/baumhard/src/font/fonts.rs::init` and confirm it does
   nothing on the second call. (If it does anything beyond the
   `OnceLock` check, the audit produces no removals — note that
   in the commit message and stop.)
2. **Remove the per-body call from suites whose fixture builders
   already call it.** The survey identified
   `bvh_descent_tests.rs` (7 sites) as a clear case where the
   shared `build_*_tree` fixture already calls `fonts::init()`,
   making the per-test call redundant.

**Critical caveat:** `do_*()` bodies that are reachable from
`benches/test_bench.rs` must keep their `fonts::init()` call
**because the benchmark binary does not run any setup before
calling them**. Per `§B8`, each `do_*` is self-contained.
Removing `fonts::init()` from a `do_*` body breaks the bench
when run from `cargo bench` even though `cargo test` still
passes (because tests run wrapped by `#[test]` functions which
Rust harnesses together).

The safe rule:

- **Inside `#[test]`-only functions** (no companion `do_*`):
  `fonts::init()` may be deleted if a sibling fixture already
  initialises.
- **Inside a `do_*`**: `fonts::init()` stays. Always. No
  exceptions.

Expected reduction: ~10–15 redundant calls in inline-test bodies,
not the 27 the survey speculated. The `do_*` discipline is more
important than the line count.

If the audit finds zero safely-removable calls (e.g. all 73 sites
are in `do_*` bodies), **skip this commit**. The duplication is
load-bearing for benchmark reuse and we leave it alone.

### §8.3 Phase E.3 — `mutator_tests.rs` and `tree_walker_tests.rs` factories (commit 9)

**Inventory:**

- `mutator_tests.rs:35,85,129` — three `do_*` functions whose
  first 12 lines are an identical "init fonts; build a 1-node
  `GfxElement`" preamble. The text and starting position differ,
  the mutation under test differs, the assertion differs.
- `tree_walker_tests.rs:194,224,248,273,306,333,377,412,441,487,511`
  — eleven `do_*` / `#[test]` functions whose first ~5 lines
  are an identical "init fonts; build a 2-node tree via `Tree::new_non_indexed`
  + `append_area`" preamble.

**Strategy:**

Inside each test file, land a small **file-local** factory:

```rust
// In mutator_tests.rs:
fn mk_element(text: &str, x: f32, y: f32) -> GfxElement {
    fonts::init();
    GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(text, x, y),
        0,
        DEFAULT_FONT_SIZE_PT, // pull from existing constant
    )
}

// In tree_walker_tests.rs:
fn two_node_model() -> (Tree<GfxElement, GfxMutator>, NodeId, NodeId) {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let a = append_area(&mut tree, /* sensible defaults */);
    let b = append_area(&mut tree, /* sensible defaults */);
    (tree, a, b)
}
```

These helpers are private to the test file (no `pub` keyword,
not part of the benchmark surface). They consolidate the
preambles without changing any `do_*` signature, so
`benches/test_bench.rs` is unaffected.

If a `do_*` body needs a different element-construction shape
(e.g. multi-line text), it keeps its inline construction. The
helper is for the **identical** preamble cases only.

**Visibility note:** these helpers stay file-local and **not
`pub`** — the benchmark file imports the `do_*` functions, not
their internal helpers, so the helpers' invocation is inlined
through the `do_*` bodies and does not need to be benchmark-
reachable on its own.

### §8.4 Acceptance criteria for Phase E

- `grep -rn "fn test_map_path" lib/baumhard/src/` returns **one**
  match (the canonical `mindmap::test_helpers::testament_map_path`).
- `grep -rn "fn synthetic_node\|fn synthetic_map\|fn synthetic_portal_edge"
  lib/baumhard/src/` returns the canonical definitions only.
- The six inline `Canvas {}` literals in `model/tests.rs` are gone
  (replaced by `blank_canvas()`).
- `./test.sh` green; total test count unchanged.
- `./test.sh --bench` green — **this is the load-bearing check
  for Phase E**, since the bench harness is the only way drift
  surfaces. If the bench harness builds and runs, the
  benchmark-reuse contract is intact.
- `cargo doc -p baumhard --no-deps` produces no warnings about
  missing doc comments on the new `pub` items in
  `mindmap::test_helpers`.
---

## §9 Verification regimen

Every commit in this plan satisfies the same checklist before
landing. `CODE_CONVENTIONS.md §11/§12` is the source of truth;
this section makes the project-specific application explicit.

### §9.1 The local gate

Before *every* commit:

```sh
./test.sh
```

This runs the full suite across both crates **and** the WASM
type-check gate (per `CLAUDE.md`'s "Common tasks"). Both must
pass. The printed test count at the end of the run is the
canary for "did we accidentally drop tests?" — note it before
each commit and confirm afterwards.

For commits in Phase E (baumhard):

```sh
./test.sh --bench
```

per `lib/baumhard/CONVENTIONS.md §B8`. The bench harness is the
only thing that catches `do_*` drift; running it at every
baumhard-side commit closes that gap.

For commits in Phase B (testament-loader unification) and Phase
C (fixture-builders), where shared code visibility is widened:

```sh
./build.sh --wasm
```

confirms `wasm32-unknown-unknown` still builds. Per
`CODE_CONVENTIONS.md §4` and `CLAUDE.md`'s dual-target status
section, the parity surface is non-negotiable.

`./test.sh --lint` is advisory and reviewed at the end of each
commit; clippy regressions get squashed in the same commit
that introduced them (per `§5`).

### §9.2 The duplication-grep audits

Per the per-phase acceptance criteria, every duplication category
has a `grep -rn` invocation that should reach exactly one
canonical definition. The full audit list, runnable as a
sanity check after the whole plan lands:

```sh
# Mandala-side audits.
grep -rn 'fn drive_throttle_over_budget' src/
grep -rn 'fn fixture_edge\b' src/  # only the throttled-interaction shared one
grep -rn 'fn test_map_path\b' src/  # one
grep -rn 'fn load_test_doc\b' src/  # one
grep -rn 'fn first_node_id\b' src/  # one (or zero; consolidated as `first_testament_node_id`)
grep -rn 'fn make_test_mutation\b\|fn make_cm\b\|fn make_test_mutation_with_timing\b' src/  # one
grep -rn 'env!("CARGO_MANIFEST_DIR").*testament.mindmap.json' src/  # one
grep -rn 'fn run\b' src/application/console/  # one (the generic dispatcher)
grep -rn 'fn .*sample_geometry\b' src/  # one definition + at most one alias

# Baumhard-side audits.
grep -rn 'fn test_map_path\b' lib/baumhard/src/  # one (testament_map_path)
grep -rn 'fn synthetic_node\b\|fn synthetic_map\b\|fn synthetic_portal_edge\b' lib/baumhard/src/  # one each
grep -rn 'Canvas {' lib/baumhard/src/mindmap/model/tests.rs  # zero new literals; old ones replaced
```

A failure of any of these greps is grounds to revert the
landing chunk and re-do it.

### §9.3 The test-count canary

At the end of every commit:

```sh
./test.sh 2>&1 | tail -3
```

prints a "passed; N tests" line. The `N` is the canary. Phase A
(throttled_interaction macro) is the only phase where the count
*could* legitimately change — if the macro emits a slightly
different set of tests than the old hand-written ones. The plan
target is **N stays exactly the same** across the entire effort.
If a chunk produces a different `N` and the discrepancy is
intentional (e.g. a duplicated test really was redundant and
absorbed), that is documented in the commit message.

### §9.4 Manual sanity passes per phase

Beyond the automated gates:

- **Phase A** — open one of the throttled-interaction files and
  verify the macro invocation reads cleanly. If the macro
  produces a `mod`-scoped namespace, confirm `cargo test
  -p mandala --lib throttled_interaction::moving_node`
  still works as a name filter.
- **Phase B** — run `cargo test -p mandala --lib console` and
  `cargo test -p mandala --lib document` independently to
  confirm the divergent-shape consolidation didn't regress
  the console tests' specific expectations.
- **Phase C** — open `app/tests.rs` and confirm the file is
  visibly shorter. The `MindNode` literal block was 40+ lines.
- **Phase D** — `cargo test -p mandala --lib console::commands`
  exercises every per-command path; confirm none of them
  regressed when their `run` helpers vanished.
- **Phase E** — `cargo bench --bench test_bench` (or
  `./test.sh --bench`) builds and runs the criterion harness;
  confirm no `unresolved import` errors against
  `mindmap::test_helpers` (would mean the `pub` visibility on
  the consolidated module is wrong).

---

## §10 Commit map

Twelve possible commits, sized so each is a session-chunk and
green on its own. The plan can deliver in fewer if commits A1+A2
or D combine cleanly, but not more — each is the smallest unit
that *makes coherent sense* on its own.

| #   | Phase | Title (commit-message starting line)                                  | Crate(s)             | Estimated diff size      |
|-----|-------|-----------------------------------------------------------------------|----------------------|--------------------------|
| 1   | A1    | Drop `moving_node`'s local `drive_throttle_over_budget`               | mandala              | ~20 lines deleted        |
| 2   | A2    | Land `trait_default_tests_for_throttled_interaction!` macro           | mandala              | +60 / −120 lines         |
| 3   | A3    | Apply throttled-interaction macro to remaining three impls            | mandala              | +20 / −180 lines         |
| 4   | B     | Funnel testament loaders through `tests_common::load_test_doc`        | mandala              | +10 / −90 lines          |
| 5   | C     | Consolidate `make_test_mutation` family + `MindNode`/`MindEdge` test fixtures | mandala      | +60 / −110 lines         |
| 6   | D     | Delete per-command `run` helpers; share color-picker geometry fixture | mandala              | +5 / −60 lines           |
| 7   | E.1   | Land `mindmap::test_helpers`; replace four `test_map_path` copies     | baumhard             | +120 / −180 lines        |
| 8   | E.2   | Audit and trim redundant `fonts::init()` calls (only outside `do_*`)  | baumhard             | −10 to −20 lines         |
| 9   | E.3   | File-local factories in `mutator_tests.rs` / `tree_walker_tests.rs`   | baumhard             | +20 / −60 lines          |

Each commit message follows the existing repo style (verified by
`git log` during the chunked plan-write; messages explain *why*,
not what the diff shows, per `§12`). Sample for #2:

> Land `trait_default_tests_for_throttled_interaction!` macro
>
> Five throttled interactions ship with five copies of the same
> trait-default tests; the only varying parts are the fixture
> constructor and the per-impl "set pending" hook. Centralise
> the test bodies in one macro so the trait-default contract
> has one home and a future sixth implementor inherits it for
> free instead of copying the scaffold.
>
> Two impls (`moving_node`, `edge_handle`) consume the macro in
> this commit so the parameterisation is exercised on both the
> delta-accumulator and snapshot-edge shapes. The remaining three
> follow in the next commit. No test count change.

### §10.1 What does NOT land in this plan

- **No new `tests-helpers/` crate.** Plan §1.4.
- **No mocking, snapshot, or pretty-print test-deps added.** Plan
  §1.5.
- **No widening `pub(super)` to `pub`** beyond what the consolidation
  strictly requires (plan §1.3). The single exception is the
  baumhard `test_helpers` module's items, which **must** be `pub`
  to satisfy the benchmark-reuse seam (`§T2.2`).
- **No `Canvas` literal in production code is touched.** The
  `blank_canvas()` helper is a test-side affordance only.
- **No reordering or restructuring of tests beyond what
  consolidation requires.** A test that ends up consumed by a
  macro keeps its semantics. A test that stays inline keeps its
  position.
- **No `// TODO` markers, no `// FIXME`.** Per `§5`.

### §10.2 What it leaves on the table

A few smaller residual duplications survive the plan because they
do not clear the three-strikes-and-obscures-intent bar:

- The per-command `fixture_doc()` thin wrappers
  (`font.rs:448`, `border/tests.rs:17`, etc.) — each is a
  one-liner that delegates to `tests_common::load_test_doc`.
  They could be deleted in favour of direct calls, but they are
  *idiom*, not *shape*: every command-test file says
  `let doc = fixture_doc()` and that local sentence reads
  cleanly. Leave them.
- `app/tests.rs::FROM_ID/TO_ID/EDGE_TYPE` constants — local to
  one test module, two-line repeat. Below the threshold.
- `select_first_edge` in `console/tests/fixtures.rs:45` —
  one definition, multiple call sites; this is the desired
  shape, not duplication.

These are noted so a future session does not re-litigate them.
The bar for "leave it" is the same bar `§7` defines for
"don't extract": three repetitions plus obscured intent.

---

## §11 Parallelism and dependency graph

The phases are independent at the file level except for the
following ordering constraints:

- **Phase B** must precede **Phase D** (Phase D's deletions
  rest on Phase B's loader being canonical).
- **Phase A** and **Phase C** are independent of each other;
  either can land first.
- **Phase E** is fully independent of the mandala-side phases
  and can start at any point.
- **Phase E.1** (test_helpers module) must precede **E.3**
  (file-local factories) only if E.3 references the new shared
  fixtures (it doesn't in the current plan, so they're
  independent).

A reasonable single-session sequencing:

```
A1 → A2 → A3 → B → C → D → E.1 → E.2 → E.3
```

A two-session split would be: mandala (A1–D) in one session,
baumhard (E.1–E.3) in another.

---

## §12 Risks and mitigations

| Risk                                                         | Mitigation                                                                  |
|--------------------------------------------------------------|------------------------------------------------------------------------------|
| Macro expansion produces unreadable test names               | The macro's emitted module name is the caller-supplied `$mod` ident, so test output reads `throttled_interaction::moving_node::drain_defaults::test_should_perform_drain_false_when_idle`. Confirm in Phase A2 review. |
| Phase B's divergent-shape consolidation hides a real difference | The `console::tests::fixtures::load_test_doc` body diverged from canonical (no cache, hand-built shell, then `build_mutation_registry()`). The consolidation must preserve the `build_mutation_registry()` step. Verify with `cargo test -p mandala --lib console`. |
| Phase C's `defaults::*` widening leaks production seams      | `defaults` is already a small file of test-friendly factory functions. `pub(crate)` only widens visibility for in-crate callers; production WASM-build surface is unchanged. `./build.sh --wasm` catches drift. |
| Phase E.2 strips `fonts::init()` from a hidden `do_*`        | Strict rule (§8.2): `do_*` bodies keep their `fonts::init()` always. Audit script: `grep -B 5 "fonts::init()" lib/baumhard/src/ | grep "pub fn do_"` to flag any candidate removal that lives in a `do_*`. |
| Phase E.1 breaks the bench harness                           | `./test.sh --bench` runs at the end of E.1 commit. If it red-lights, revert and split the commit into "introduce module" + "migrate fixtures" subcommits. |
| The macro-generated tests don't appear in `cargo test --list`| `macro_rules!` expansions of `#[test]` attributes do appear in `cargo test --list`, verified by inspecting `cargo test --list 2>&1 | grep should_perform_drain` after Phase A. |
| Visibility tightening accidentally widens scope              | The plan's §1.3 "narrowest visibility" rule is enforced commit-by-commit. Reviewers grep for `pub(crate)` and `pub fn` introductions and confirm the call graph requires them. |
| Renaming a `do_*` breaks `benches/test_bench.rs`             | The plan does NOT rename any `do_*`. Phase E adds shared fixtures *under* the `do_*` bodies; signatures stay identical. |

---

## §13 Done criteria

The plan is fully delivered when:

1. All twelve audit `grep` queries from §9.2 resolve to their
   stated canonical-only outcomes.
2. `./test.sh` is green.
3. `./test.sh --bench` is green.
4. `./build.sh --wasm` is green.
5. `cargo doc -p baumhard --no-deps` produces no warnings about
   missing doc comments.
6. The throttled-interaction module's test surface, viewed in a
   single editor pane, fits comfortably without scrolling — the
   visible improvement the user asked for.
7. The total test count (the canary printed by `./test.sh`) is
   unchanged from the pre-refactor baseline.
8. No new files in the workspace that aren't strictly required.
   (Concrete prediction: **one** new file, namely
   `lib/baumhard/src/mindmap/test_helpers.rs`. Everything else
   is editing existing files.)

---

## §14 Closing posture

This refactor pays down test-side duplication that built up
because each new throttled interaction (and each new console
command, and each new mindmap-layer test module) was authored
*by copying its sibling*. The conventions
(`CODE_CONVENTIONS.md`, `TEST_CONVENTIONS.md`,
`lib/baumhard/CONVENTIONS.md`) already forbid this — every
green commit is supposed to *improve* the code (`§5`). The job
of this plan is to make one explicit pass over the surface and
close the gaps so the next session inherits a cleaner surface
and the benchmark-reuse / fixture-cache / cross-platform-parity
disciplines are reachable through one canonical seam each.

The standard, per `§0`, is canonical or exemplary. The standard
is also that the foundation must be pristine. We exorcise the
ghetto pattern, leave the seams cleaner than we found them, and
then go build the next thing.
