# WASM Convergence

This document is the porting guide for unifying the WASM and native
input pipelines. It exists because Mandala targets both platforms
as first-class deployments (per `CODE_CONVENTIONS.md §4`), but
today's WASM target has only a curated subset of the modals,
gestures, and Actions that native ships.

If you're picking this work up, **start here, then read in order**:
[`CONCEPTS.md §5 "Action dispatch"`](./CONCEPTS.md), the
`Action::wasm_compatibility` method in
[`src/application/keybinds/action.rs`](./src/application/keybinds/action.rs),
and [`src/application/app/run_wasm.rs`](./src/application/app/run_wasm.rs).

## The current shape

**Native** (`src/application/app/`) has:
- `dispatch_action(action, &mut InputHandlerContext, hit)` — the
  single funnel every Action body runs through.
- `dispatch_macro(macro_id, ctx)` — same shape for macros.
- `dispatch_custom_mutation_for_key` — same shape for keybind-
  triggered custom mutations.
- A 21-field `InputHandlerContext` covering every modal /
  state-machine / per-frame field the dispatch arms might touch.
- A `MacroRegistry` with App / User / Map tiers loaded.
- `bundle!()` macros in `event_keyboard.rs` and `event_mouse_click.rs`
  that rebuild the context bundle for the dispatcher.

**WASM** (`src/application/app/run_wasm.rs`) has:
- Its own `WasmInputState` struct with 9 fields (a strict subset of
  the native context).
- An inline `match action { ... }` block for keyboard input that
  hardcodes which Actions it knows how to handle.
- An inline `match &click_hit { ... }` ladder for double-click —
  not routed through `dispatch_action`.
- No `MacroRegistry`. Macros silently no-op in the browser.
- No `dispatch_action`, `dispatch_macro`, `dispatch_custom_mutation_for_key`.

The asymmetry is the **convergence gap**: the same Action variant
behaves differently across targets, and adding a new variant
requires touching both files.

## The convergence target

Long-term: a single `dispatch_action` callable from both targets,
with WASM gradually gaining the missing systems so more Actions
flip from `NativeOnly` to `Compatible`.

The `Action::wasm_compatibility(&self) -> WasmCompatibility` method
([`src/application/keybinds/action.rs`](./src/application/keybinds/action.rs))
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

## Three porting tracks (do them in any order)

### Track A — port a NativeOnly Action

When you want a specific feature in the browser. Pick an Action
classified `NativeOnly` (e.g. `Action::OpenConsole`).

1. Decide whether to port the underlying system (full console on
   WASM) or surface a WASM-shaped equivalent (e.g. a `<dialog>`
   element instead of an in-canvas overlay).
2. Add the corresponding state to `WasmInputState` in
   `run_wasm.rs`.
3. Open the matching dispatch arm in
   `src/application/app/dispatch.rs` and inspect what `ctx` fields
   it touches. Audit: does the WASM-side state shape match
   closely enough that the SAME arm body works?
4. If yes: extend `WasmInputState` to expose those fields the same
   way `InputHandlerContext` does, and write a `WasmInputContext`
   adapter that satisfies the same field-access pattern.
5. Flip the Action's `wasm_compatibility` classification to
   `Compatible`.
6. Call `dispatch_action(a, &mut wasm_bundle, hit)` from
   `run_wasm.rs` instead of the inline match arm.
7. Remove the inline arm from `run_wasm.rs`'s match.
8. Update the test in `src/application/keybinds/tests.rs`
   (search `test_wasm_compatibility_*`).

### Track B — port the macro registry

WASM currently has no `MacroRegistry`. Once it does, every macro
that's already `Compatible` works in the browser.

1. Decide where the user-tier loader reads from on WASM. There's
   no `~/.config/mandala/macros.json` in a browser; the natural
   shape parallels the keybind loader: `?macros=<json>` query
   param, or `localStorage["mandala_macros"]`. See
   `src/application/keybinds/platform_web.rs` for the existing
   pattern.
2. Add `loader::load_user_macros_wasm()` parallel to the native
   `load_user_macros`.
3. Reuse `loader::load_app_macros()` — it's `include_str!`-based
   so it works on both targets unchanged.
4. Reuse `loader::rebuild_map_macros(registry, doc)` — it's
   pure logic, no platform-specific I/O.
5. Add `macros: MacroRegistry` to `WasmInputState`. Build it at
   startup in `run_wasm::run`.
6. When the document loads (and re-loads via `?map=`), call
   `rebuild_map_macros(macros, doc)`.
7. After dispatching keys to `dispatch_action` per Track A, also
   dispatch through the macro path — see
   `src/application/app/event_keyboard.rs` for the resolution
   order: Action → Macro → CustomMutation.
8. **Do not skip the privilege gate.** `dispatch_macro` enforces
   `MacroSource::allows_console_line` and `allows_action`. If
   you reimplement the dispatch loop on WASM, port the gate too —
   `src/application/app/dispatch.rs::dispatch_macro` is the
   reference. A hostile mindmap loaded in the browser is the
   primary threat model the gate exists to defend against.

### Track C — unify the bundle / context type

Eventually the two `bundle!()` macros and the `WasmInputState`
should converge. Two viable shapes:

**Shape 1: shared `InputContextCore` + native-only extension.**
Define a new `InputContextCore` containing the 9 fields both
targets have. Native's `InputHandlerContext` keeps its 21-field
struct but embeds `InputContextCore` plus 12 native-only fields.
WASM constructs `InputContextCore` directly. `dispatch_action`'s
signature changes to `&mut InputContextCore`; arms that need
native-only state get gated behind a separate
`fn dispatch_native_only_action(action, &mut InputHandlerContext)`.

**Shape 2: trait-based context.** Define
`trait DispatchableContext { fn document_mut(&mut self) -> ...; ... }`
with accessors for every field. `InputHandlerContext` implements
the full trait; `WasmInputState` implements a subset. Each
dispatch arm requires only the accessors it actually uses; the
borrow checker handles the split borrows. More flexible, more
trait surface to maintain.

Shape 1 is closer to the current code and likely the better
landing point. Shape 2 is more idiomatic Rust but is a bigger
diff.

### Track-D meta — keep the privilege model intact

Whatever shape Track C takes, the macro privilege gate
(`MacroSource::allows_console_line`, `allows_action`, fail-closed
in `dispatch_macro`) MUST remain in the dispatch path on both
targets. The `WasmCompatibility` classification is orthogonal —
a `Compatible` Action might still be `NativeOnly`-equivalent for
non-User macro tiers (e.g. `SaveDocument` is `NativeOnly` only
because filesystem; a future cloud-save would be `Compatible`,
but it must STILL be denylisted on `MacroSource::allows_action`
because hostile mindmaps shouldn't be able to invoke it).

## What's deferred today (and tracked in TODO.md)

- Full `dispatch_action` callable from WASM. Track A or C.
- `MacroRegistry` on WASM. Track B.
- The inline label / portal-text editors and the color picker on
  WASM. Track A on individual Actions.
- The console on WASM. Track A.
- `AppMode` (Reparent / Connect) on WASM. Track A.
- `DragState` / continuous-drag gestures (`PanCanvas`) on WASM.
  Track A — note that WASM has its own `pending_click` mechanism
  that may serve as the basis.

## Smoke-testing the boundary

When you flip an Action from `NativeOnly` to `Compatible`, the
test in `src/application/keybinds/tests.rs` (e.g.
`test_wasm_compatibility_console_modals_are_native_only`) starts
failing — that's the signal to update the test alongside the
classification. For the new behaviour itself, the existing test
suite covers the dispatch arm via the native path; the WASM
target has manual smoke-test boilerplate in `run_wasm.rs` but
no automated coverage (there's no headless WASM browser harness
today, per `TEST_CONVENTIONS.md §T9`).

## Reading order for the impatient

1. [`src/application/keybinds/action.rs`](./src/application/keybinds/action.rs) —
   `Action::wasm_compatibility` is the API contract.
2. [`src/application/app/dispatch.rs`](./src/application/app/dispatch.rs) —
   the native dispatch funnel arms are the reference implementation.
3. [`src/application/app/input_context.rs`](./src/application/app/input_context.rs) —
   the 21-field bundle every native arm reads.
4. [`src/application/app/run_wasm.rs`](./src/application/app/run_wasm.rs) —
   the WASM event loop with its inline match blocks. This file
   shrinks dramatically as Track A ports land.
5. [`format/macros.md`](./format/macros.md) — the privilege model
   that Track B must preserve.
