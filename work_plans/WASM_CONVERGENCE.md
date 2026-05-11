# WASM Convergence

This document is the porting guide for unifying the WASM and native
input pipelines. Mandala targets both platforms as first-class deployments
(per `CODE_CONVENTIONS.md §4`); today's WASM target has a curated subset
of the modals, gestures, and Actions that native ships.

If you're picking this work up, **start here, then read in order**:
[`CONCEPTS.md §5 "Action dispatch"`](../CONCEPTS.md), the
`Action::wasm_compatibility` method in
[`src/application/keybinds/action/mod.rs`](../src/application/keybinds/action/mod.rs),
[`src/application/app/dispatch/native.rs`](../src/application/app/dispatch/native.rs)
(the reference implementation), and
[`src/application/app/run_wasm/`](./src/application/app/run_wasm/).

## The current shape

**Native** (`src/application/app/`) has:
- `dispatch_action(action, &mut InputHandlerContext, hit)` — the
  single funnel every Action body runs through.
- `dispatch_macro(macro_id, ctx)` — same shape for macros.
- `dispatch_custom_mutation_for_key` — same shape for keybind-
  triggered custom mutations.
- A 21-field `InputHandlerContext` covering every modal /
  state-machine / per-frame field the dispatch arms might touch.
- A `MacroRegistry` with App / User / Map / Inline tiers loaded.

**WASM** (`src/application/app/run_wasm/`) has:
- Its own `WasmInputState` struct holding the cross-platform fields
  plus a `MacroRegistry`.
- An inline `match action { ... }` block for keyboard input where
  every Compatible Action arm calls into the shared
  `dispatch::cross_dispatch` helper module.
- An inline `match &click_hit { ... }` ladder for double-click —
  the largest remaining Track-A duplication. Not yet routed through
  `dispatch_action`.

Tracks B (macro registry) and C (full context-type unification)
landed; both targets dispatch every Compatible Action through the
same `dispatch::action_core::dispatch_compatible` function over an
`InputContextCore<'a>` view. WASM has a `MacroRegistry` populated
at startup with the same loader/parser code paths native uses.
Track A.3 — lifting per-arm bodies into `dispatch::cross_dispatch` —
is the recommended path for any new Compatible Action.

## The convergence target

Long-term: a single `dispatch_action` callable from both targets,
with WASM gradually gaining the missing systems so more Actions
flip from `NativeOnly` to `Compatible`.

The `Action::wasm_compatibility(&self) -> WasmCompatibility` method
([`src/application/keybinds/action/mod.rs`](../src/application/keybinds/action/mod.rs))
is the typed API surface. It classifies every Action as
`Compatible` or `NativeOnly`. The match is exhaustive on `Action`
(an open enum via `#[non_exhaustive]`); a developer adding a new
variant is forced by the compiler to make the call.

`WasmCompatibility::Compatible` means "this Action's body only
touches state that exists on both targets" (`MindMapDocument`,
`Renderer`, `text_edit_state`, mouse gestures, the macro registry).
`WasmCompatibility::NativeOnly` means "this Action's body needs
native-only state" (console, color picker, label editor, AppMode,
DragState, filesystem). The doc-comment on `wasm_compatibility`
spells out the rules in detail.

## Track A — port an Action to WASM

When you want a specific feature in the browser, or you've added
a new Compatible variant and need to wire it through both
dispatchers. **Three paths**, in order of preference:

- **Path A.3 — partial Track C (preferred for Compatible Actions).**
  Add a per-action helper to `dispatch/cross_dispatch.rs` that takes
  the typed payload + a `RebuildContext`. Both dispatchers call the
  same function — no mirror tax. This is what every camera /
  selection / FPS / parametric Compatible arm does today (see
  the `apply_zoom_step`, `apply_select_all`, `apply_set_color_axis`
  shapes for templates).
- **Path A.1 — inline mirror arms.** For Compatible Actions whose
  bodies need state only one side has, OR for NativeOnly Actions
  you want partial WASM coverage of, add an inline arm to
  `run_wasm/` that touches WASM-shaped state. Dispatch logic
  duplicates until A.3 lift consolidates.
- **Path A.2 — full Track D.** Once both targets share `&mut self`
  shape on event handlers, route through `dispatch_action` directly.
  Cleanest endpoint, biggest refactor. See "Per-arm event-handler
  shape divergence" below for the open question.

### Steps for Path A.1

1. Decide whether to port the underlying system (full console on
   WASM) or surface a WASM-shaped equivalent (e.g. a `<dialog>`
   element instead of an in-canvas overlay).
2. Add the corresponding state to `WasmInputState`.
3. Open the matching native dispatch arm in
   `src/application/app/dispatch/native.rs` to understand the body.
4. Write a parallel arm in `run_wasm/`'s `match a { ... }` block
   that does the same thing against `WasmInputState`. Comment with
   `// MIRROR OF dispatch/native.rs::Action::Foo arm — keep in
   sync until A.3 lift consolidates.`
5. Flip the Action's `wasm_compatibility` classification to
   `Compatible`. Update the corresponding test in
   `src/application/keybinds/tests.rs`.

## Track-D meta — keep the privilege model intact

The macro privilege gate (`MacroSource::allows_console_line`,
`allows_action`, fail-closed in `dispatch_macro`) MUST remain
single-sourced on both targets. The
[`format/macros.md`](../format/macros.md) "Privilege model"
section is the authoritative spec; the implementation lives in
`src/application/macros/mod.rs` and the enforcement loop in
`src/application/app/dispatch/macro_core.rs`. The
`WasmCompatibility` classification is orthogonal — a
`Compatible` Action might still be denylisted by
`MacroSource::allows_action` for non-User macros (e.g.
`Action::SaveDocument` would be `Compatible` once WASM gains a
save path, but it'd still be in the denylist because hostile
mindmaps shouldn't invoke it).

`MacroSource::allows_action` and `allows_console_line` live in
`src/application/macros/mod.rs` (cross-platform). The fail-closed
enforcement loop is in `dispatch::macro_core::dispatch_macro`,
abstracted over a `MacroDispatchTarget` trait so native and WASM
share the body byte-for-byte.

Re-implementing the privilege check inline is **forbidden** —
it's the threat-model defence and must be single-sourced. A
forked enforcement copy would silently drift when a future
contributor adds an Action to the denylist.

## What's deferred today (and tracked in TODO.md)

- The inline label / portal-text editors and the color picker on
  WASM. Track A on individual Actions.
- The console on WASM. Track A.
- `AppMode` (Reparent / Connect) on WASM. Track A.
- `DragState` / continuous-drag gestures (`PanCanvas`) on WASM.
  Track A — note that WASM has its own `pending_click` mechanism
  that may serve as the basis.
- Filesystem on WASM (`OpenDocument` / `SaveDocumentAs` /
  `NewDocumentAt` parametric Action variants stay `NativeOnly`
  pending a chosen storage strategy).
- Touch / IME / Focused input event arms — the catch-all in
  `WasmApp::handle_window_event` documents these by name; each
  needs its own `event_*.rs` sibling once wired. Touch is
  mobile-budget-binding (§4); IME is required for non-Latin
  text editing in the inline node-text editor.
- Maptool migration on WASM (`maptool convert --sections` is
  native-only by construction; a browser-only authoring flow
  that loads a legacy map needs an in-app migration path).

## Per-arm event-handler shape divergence

Native handlers are free functions taking `&mut
InputHandlerContext<'_>`. WASM handlers are inherent methods on
`WasmApp` because the `Rc<RefCell<Option<…>>>` cell projection
forces a `&mut self` shape. Track D's full convergence will
need to either remove the cells (convert `WasmApp` to own its
state directly) or accept the method shape on both sides. New
WASM event handlers added before Track D should follow the
method pattern.

## Smoke-testing the boundary

When you flip an Action from `NativeOnly` to `Compatible`, the
test in `src/application/keybinds/tests.rs` starts failing —
that's the signal to update the test alongside the classification.
The existing test suite covers the dispatch arm via the native
path; the WASM target has no headless harness
(`TEST_CONVENTIONS.md §T9`) — manual smoke via `trunk serve`.
