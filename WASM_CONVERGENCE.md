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

## Three porting tracks

The tracks have soft dependencies. **Track C is the prerequisite
for Track A landing through `dispatch_action`** — the native
`dispatch_action` takes `&mut InputHandlerContext` (21 native-only
fields). Until a shared context type exists, individual Action
ports under Track A must add inline arms to `run_wasm.rs` rather
than route through the unified dispatcher. **Track B (the macro
registry) can land independently of A and C** — the registry's
data and resolver are self-contained — but does require the
prerequisite step 0 below.

### Track A — port a NativeOnly Action

When you want a specific feature in the browser. Pick an Action
classified `NativeOnly` (e.g. `Action::OpenConsole`).

**Two paths today, depending on Track C's status:**

- **Path A1 (Track C not yet landed — current state).** Port the
  Action by adding an inline arm to `run_wasm.rs` that touches
  WASM-shaped state. The dispatch logic is duplicated between
  native (`dispatch.rs`) and WASM (`run_wasm.rs`) until Track C
  consolidates. This is what `run_wasm.rs`'s existing arms (Undo,
  CreateOrphanNode, OrphanSelection, DeleteSelection,
  EditSelection-Single-only) do today.
- **Path A2 (Track C landed).** Route the Action through
  `dispatch_action` once both targets share a context type. This
  is the cleaner endpoint.

**Steps for Path A1:**

1. Decide whether to port the underlying system (full console on
   WASM) or surface a WASM-shaped equivalent (e.g. a `<dialog>`
   element instead of an in-canvas overlay).
2. Add the corresponding state to `WasmInputState` in
   `run_wasm.rs`.
3. Open the matching native dispatch arm in
   `src/application/app/dispatch.rs` to understand the body.
4. Write a parallel arm in `run_wasm.rs`'s `match a { ... }` block
   that does the same thing against `WasmInputState`. Comment with
   `// MIRROR OF dispatch.rs::Action::Foo arm — keep in sync until
   Track C consolidates.`
5. Flip the Action's `wasm_compatibility` classification to
   `Compatible`. Update the corresponding test in
   `src/application/keybinds/tests.rs`.

**Steps for Path A2 (after Track C):**

1. Decide whether to port the underlying system or surface a
   WASM-shaped equivalent.
2. Add the state to whatever shared context Track C lands.
3. Audit the dispatch arm: does it work unchanged on the shared
   context, or does it need branching on the platform?
4. Flip the classification, update the test.
5. Remove the inline arm from `run_wasm.rs` (if it exists from
   a prior Path A1 port).

### Track B — port the macro registry

WASM currently has no `MacroRegistry`. Once it does, every macro
whose Action steps are already `Compatible` works in the browser.

**Step 0 — prerequisite: lift the cfg gate.**
`src/application/macros/loader.rs` is `#![cfg(not(target_arch =
"wasm32"))]`-gated at module level (line 16), and
`src/application/macros/mod.rs:18` only declares `pub mod loader`
under the same cfg. Today no part of the loader compiles on WASM.
Audit each function for `std::fs` / native-only API usage:

| Function | Status | Action |
|---|---|---|
| `load_app_macros` | `include_str!`-based, pure | move out from under the cfg |
| `parse_map_macros` | pure | same |
| `rebuild_map_macros` | pure (calls `parse_map_macros` + registry methods) | same |
| `parse_inline_macros` | pure | same |
| `rebuild_inline_macros` | pure | same |
| `load_user_macros` | reads `~/.config/...` via `std::fs` | keeps the cfg gate (or grows a WASM-side sibling) |

The cleanest shape: split `loader.rs` into a portable file
(everything except `load_user_macros`) and a `cfg`-gated
`platform_desktop.rs` for the native filesystem reader, parallel
to `keybinds/platform_desktop.rs` and the mutations loader. Then
add `platform_web.rs` for WASM's `?macros=` / `localStorage`
loader.

**Once Step 0 is done:**

1. Decide where the user-tier loader reads from on WASM. There's
   no `~/.config/mandala/macros.json` in a browser; the natural
   shape parallels the keybind loader: `?macros=<json>` query
   param, or `localStorage["mandala_macros"]`. See
   `src/application/keybinds/platform_web.rs` for the existing
   pattern.
2. Add `loader::platform_web::load_user_macros()` parallel to
   the native loader.
3. Reuse `loader::load_app_macros()` — `include_str!`-based.
4. Reuse `loader::rebuild_map_macros` and
   `loader::rebuild_inline_macros` — both pure once the cfg gate
   is lifted.
5. Add `macros: MacroRegistry` to `WasmInputState`. Build it at
   startup in `run_wasm::run`.
6. When the document loads (and re-loads via `?map=`), call
   `rebuild_map_macros(macros, doc)` and
   `rebuild_inline_macros(macros, doc)`.
7. Dispatch macros after the WASM keyboard handler's
   `Action::is_some()` branch. See `event_keyboard.rs:347-378`
   for the native chain (Action → Macro → CustomMutation).
8. **Do not skip the privilege gate.** See Track-D below.

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

**Scaffolding posture.** This branch ships ZERO scaffolding for
Track C — no stub `InputContextCore` struct, no
`DispatchableContext` trait. The choice between Shape 1 and
Shape 2 is left to the contributor who lands the refactor; pre-
committing to one shape via a stub would close off the other.
Track A path A1 is the working alternative until Track C is
chosen and built.

### Track-D meta — keep the privilege model intact

The macro privilege gate (`MacroSource::allows_console_line`,
`allows_action`, fail-closed in `dispatch_macro`) MUST remain
single-sourced on both targets. The `WasmCompatibility`
classification is orthogonal — a `Compatible` Action might still
be denylisted by `MacroSource::allows_action` for non-User
macros (e.g. `Action::SaveDocument` would be `Compatible` once
WASM gains a save path, but it'd still be in the denylist
because hostile mindmaps shouldn't invoke it).

**Where the gate lives today, and where it must stay.**
`MacroSource::allows_action` and `allows_console_line` live in
`src/application/macros/mod.rs` — these methods are NOT cfg-
gated; they compile on both targets. The fail-closed enforcement
loop, however, is in `dispatch::dispatch_macro` at
`src/application/app/dispatch.rs`, and the *entire* `dispatch.rs`
module is `#![cfg(not(target_arch = "wasm32"))]`-gated at line 9.

When you implement WASM macro dispatch, **do NOT re-implement the
`allows_action` / `allows_console_line` checks inline.** Two
acceptable shapes:

- **(a) Lift the cfg gate off `dispatch.rs`'s module declaration**
  and gate individual native-only arms instead. The privilege-
  enforcement code becomes cross-platform automatically.
- **(b) Extract `dispatch_macro` and its enforcement loop into
  `dispatch_macro_core.rs`** (cross-platform), leaving the
  Action-arm dispatcher gated. WASM imports the core module
  unchanged.

Re-implementing the privilege check inline is **forbidden** —
it's the threat-model defence and must be single-sourced. A
forked enforcement copy would silently drift when a future
contributor adds an Action to the denylist (`mod.rs:91-114`).

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
