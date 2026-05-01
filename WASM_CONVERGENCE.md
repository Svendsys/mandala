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
- A `MacroRegistry` with App / User / Map / Inline tiers loaded.
- Input handlers that take `&mut InputHandlerContext<'_>` directly
  and forward to `dispatch_action` without re-bundling.

**WASM** (`src/application/app/run_wasm.rs`) has:
- Its own `WasmInputState` struct with 9 fields (a strict subset of
  the native context).
- An inline `match action { ... }` block for keyboard input where
  every Compatible Action arm is a thin call into the shared
  `dispatch::cross_dispatch` helper module. Bodies that pre-Track-A had
  drift (Undo missing `fast_forward_animations`, e.g.) now share
  one source of truth with the native dispatcher. The mixed-branch
  Actions `EditSelection` / `EditSelectionClean` route their
  Compatible Single branch through `apply_open_text_edit_on_single`;
  the EdgeLabel / Portal branches are NativeOnly and don't fire on
  WASM.
- An inline `match &click_hit { ... }` ladder for double-click —
  not routed through `dispatch_action`.
- No `MacroRegistry`. Macros silently no-op in the browser.
- No `dispatch_action`, `dispatch_macro`, `dispatch_custom_mutation_for_key`.

The asymmetry is shrinking — Track A has folded camera, selection,
FPS, and the 20 parametric Compatible arms into shared helpers in
`src/application/app/dispatch/cross_dispatch.rs`. Both dispatchers call the
same per-action functions, so adding a new Compatible variant now
requires writing the body once (in `dispatch::cross_dispatch`), then a thin
call from each side. Tracks B (macro registry) and C (full
context-type unification) remain.

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

The tracks have soft dependencies. **Track A.3 (partial Track C)
is now the recommended path** for any new Compatible Action: lift
the body into `src/application/app/dispatch/cross_dispatch.rs` once, then
both dispatchers call the same helper. **Track B (the macro
registry) can land independently of A and C** — the registry's
data and resolver are self-contained — but does require the
prerequisite step 0 below.

### Track A — port an Action to WASM

When you want a specific feature in the browser, or you've added
a new Compatible variant and need to wire it through both
dispatchers. **Three paths**, in order of preference:

- **Path A.3 — partial Track C (preferred for Compatible Actions).**
  Add a per-action helper to `dispatch/cross_dispatch.rs` that takes the
  typed payload + a `RebuildContext`. Both dispatchers call the
  same function — no mirror tax. This is what every camera /
  selection / FPS / parametric Compatible arm does today (see
  the `apply_zoom_step`, `apply_select_all`, `apply_set_color_axis`
  shapes for templates). A new Compatible Action variant reaches
  WASM in one helper + one fan-out arm extension on each side.
- **Path A.1 — inline mirror arms.** For Compatible Actions whose
  bodies need state only one side has, OR for NativeOnly Actions
  you want partial WASM coverage of, add an inline arm to
  `run_wasm.rs` that touches WASM-shaped state. Dispatch logic
  duplicates until Track C consolidates. The existing
  `Action::Undo`, `CreateOrphanNode`, `OrphanSelection`,
  `DeleteSelection`, and `EditSelection*`-Single arms in
  `run_wasm.rs` are A.1-shape today (Track A.3 lift would unblock
  most of them).
- **Path A.2 — full Track C.** Once both targets share a context
  type, route through `dispatch_action` directly. Cleanest
  endpoint, biggest refactor.

**Steps for Path A1:**

1. Decide whether to port the underlying system (full console on
   WASM) or surface a WASM-shaped equivalent (e.g. a `<dialog>`
   element instead of an in-canvas overlay).
2. Add the corresponding state to `WasmInputState` in
   `run_wasm.rs`.
3. Open the matching native dispatch arm in
   `src/application/app/dispatch/native.rs` to understand the body.
4. Write a parallel arm in `run_wasm.rs`'s `match a { ... }` block
   that does the same thing against `WasmInputState`. Comment with
   `// MIRROR OF dispatch/native.rs::Action::Foo arm — keep in
   sync until Track C consolidates.`
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

### Track B — port the macro registry — **SHIPPED**

Track B landed in 6 commits. WASM now has a 4-tier `MacroRegistry`
populated at startup, and key bindings to macro ids fire through
the same Action → Macro chain native uses.

**Architecture summary:**

- **Loader directory** at `src/application/macros/loader/` —
  `mod.rs` holds the portable functions (`load_app_macros`,
  `parse_user_macros_json`, `parse_map_macros`,
  `rebuild_map_macros`, `parse_inline_macros`,
  `rebuild_inline_macros`, `rebuild_document_macros`).
  `platform_desktop.rs` keeps the `std::fs` reader for the
  XDG path. `platform_web.rs` reads `?macros=<urlencoded-json>`
  > `localStorage["mandala_macros"]` > empty. Cfg-routed
  `pub use` at the top of `mod.rs` exposes a single
  `loader::load_user_macros()` symbol on both targets.

- **`WasmInputState` promoted to module-level** in
  `run_wasm.rs` (was closure-local inside `pub(super) fn run`).
  Adds a `macros: MacroRegistry` field. Built at the same
  spawn_local init site as the document — App + User from
  loader, Map + Inline from `rebuild_document_macros`. Mirrors
  `run_native_init.rs:117-142` shape so cross-target log
  triage stays uniform.

- **`dispatch_macro` extracted** to
  `src/application/app/dispatch/macro_core.rs` (cross-platform,
  no cfg). Step loop + privilege gate are abstracted over a
  `MacroDispatchTarget` trait so native and WASM share the
  body byte-for-byte. **Re-implementing the loop on either
  target is forbidden** — see Track-D below for why this is
  the threat-model defence. Native impl is
  `dispatch::NativeMacroDispatchTarget` wrapping
  `&mut InputHandlerContext`; WASM impl is
  `run_wasm::WasmMacroDispatchTarget` wrapping
  `&mut WasmInputState + &mut Renderer`.

- **WASM keyboard fall-through** at the keyboard handler:
  after `keybinds.action_for_context` returns `None`,
  `keybinds.macro_for(...)` is consulted; on hit
  `dispatch::macro_core::dispatch_macro` runs the macro through
  the trait impl. Mirrors native's `event_keyboard.rs:271-310`
  Action → Macro → (CustomMutation tier on native; macros only
  on WASM today).

- **`apply_keybind_custom_mutation` lifted** from native dispatch
  (cfg-gated) to `dispatch/cross_dispatch.rs` so the WASM macro
  target can reach the same animation-aware apply +
  `apply_document_actions` envelope native uses. Re-exported from
  `dispatch/mod.rs` (as `dispatch::apply_keybind_custom_mutation`)
  for the existing `document/tests_mutations` import.

- **`MacroStep::ConsoleLine` on WASM** — User-tier logs
  `warn!` and skips (the macro continues with the next step).
  No console runtime exists in the browser; fail-closed-abort
  would surprise users copy-pasting their `macros.json` from
  desktop into `?macros=`. Non-User tiers still
  fail-closed-abort identically to native — the privilege gate
  rejects ConsoleLine from `App` / `Map` / `Inline` tiers
  before this method is called. See `format/macros.md`
  § "ConsoleLine on WASM".

**User-facing invocation on WASM:**
```text
http://localhost:8080/?map=path&keybinds={"macro_bindings":{"Ctrl+G":"my-macro"}}&macros=[{"id":"my-macro","steps":[{"kind":"Action","action":"ZoomIn"},{"kind":"Action","action":"ZoomIn"}]}]
```

**Test coverage:**
- 9 mock-target tests in `dispatch::macro_core::tests` exercise
  the privilege gate at the actual loop body (not just the
  per-step simulator).
- `loader::tests` covers `parse_user_macros_json` (the shared
  parsing seam both targets call) — pinned malformed-returns-err,
  empty-input, valid-array round-trip.
- WASM-side wiring has no headless test harness
  (`TEST_CONVENTIONS.md §T9`); manual smoke via `trunk serve`
  with the URL form above.

### Track C — unify the context type — **SHIPPED**

Track C landed in 4 commits. Both targets now dispatch every
Compatible Action through the same cross-platform
`dispatch::action_core::dispatch_compatible` function; the
WASM-only `dispatch_compatible_action_wasm` shim is deleted.

**Architecture summary:**

- **`InputContextCore<'a>`** at `src/application/app/input_context_core.rs`
  — cross-platform struct holding the 11 fields both native's
  `InputHandlerContext` and WASM's `WasmInputState` carry
  (document, mindmap_tree, app_scene, renderer, scene_cache,
  text_edit_state, last_click, cursor_pos, modifiers, keybinds,
  macros). `document` is `Option<&'a mut MindMapDocument>` so
  both ownership shapes (native's `Option`, WASM's owned-by-
  value) construct without ownership shuffles.

- **`NativeContextExt<'a>`** sibling — native-only struct holding
  the 10 fields the browser doesn't have (drag_state, app_mode,
  console_state, console_history, label_edit_state,
  portal_text_edit_state, color_picker_state, hovered_node,
  cursor_is_hand, picker_hover). cfg-gated to native.

- **`InputHandlerContext::split_borrow(&mut self) -> (InputContextCore,
  NativeContextExt)`** — re-borrows the 21 fields into the two
  views with shorter lifetime. Native callers split before
  calling the cross-platform dispatcher.

- **`dispatch::action_core::dispatch_compatible(&Action, &mut InputContextCore)
  -> DispatchOutcome`** — the canonical cross-platform dispatcher.
  Handles every Compatible-classified Action arm (Document-
  lifecycle, camera/zoom, FPS, selection nav, parametric
  mutators) via `dispatch::cross_dispatch::apply_*` helpers. Returns
  `Handled` when the body fired; `Unhandled` for NativeOnly
  variants (caller's fall-through runs them) and for mixed-
  branch Actions whose cross-platform slice didn't apply.

- **Mixed-branch handling** — `Action::CancelMode` and
  `Action::EditSelection*` have NativeOnly branches but
  cross-platform slices. The cross-platform dispatcher runs the
  slice (clear `last_click` for CancelMode; call
  `apply_open_text_edit_on_single` for EditSelection*-Single)
  and returns `Unhandled` so native fall-through runs the
  residual arm body (AppMode clearing; EdgeLabel/Portal editor
  open). Same effective behaviour as pre-Track-C native.

- **Native shim** — `dispatch::dispatch_action` adds a 9-line
  delegation at the top: split context, call
  `dispatch_compatible`, return early on Handled, otherwise
  fall through to the existing match for native-only arms.
  Compatible arms in the existing match are now unreachable
  (cross-platform dispatcher handles them first); they're left
  in place as harmless dead code, can be cleaned in a follow-up.

- **WASM caller** — `WasmInputState::input_context_core(&mut self,
  &mut Renderer, &ResolvedKeybinds)` builds the cross-platform
  view. Both the keyboard handler and `WasmMacroDispatchTarget::dispatch_action`
  call `dispatch_compatible(...)` against it. The WASM-only
  `dispatch_compatible_action_wasm` (~320 LoC) is deleted.

**The 4 commits:**
1. **C1** `b60569a` — introduce `InputContextCore` +
   `NativeContextExt` + `split_borrow` (additive, no behaviour
   change).
2. **C2** `37c2897` — cross-platform `dispatch_compatible` +
   native delegation.
3. **C3** `1fd2eeb` — wire WASM at unified dispatcher; delete
   `dispatch_compatible_action_wasm` shim.
4. **C4** *(this commit)* — docs + TODO.md.

**Why a struct, not a trait** (vs Track B's `MacroDispatchTarget`):
the macro dispatcher's loop runs a few times per gesture and
each step is a single dispatch — a virtual call there is
cheap. `dispatch_compatible` runs on every keystroke and arms
need split borrows across multiple disjoint fields (renderer +
document + mindmap_tree + app_scene simultaneously). A trait
with `&mut self` accessors would close over the whole context
on every call; the concrete struct with split borrows tracks
field-level borrows correctly.

**Native-dead-arm cleanup shipped.** The now-unreachable Compatible
arms in `dispatch/native.rs::dispatch_action`'s match (Undo, ZoomIn,
all parametric `Set*`, etc.) were removed in a follow-up commit —
they were unreachable after Track C's delegation shim, so the
removal preserved behaviour. The match now contains only NativeOnly
arms (Console / Picker / AppMode / EditOpen / Save / DoubleClick /
filesystem / PanCanvas / LabelEditCursor*), the mixed-branch native
residuals, and the mouse-with-`DispatchHit` branch of
`CreateOrphanNodeAndEdit` (the keyboard branch is in
`dispatch_compatible`).

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
loop is in `dispatch::macro_core::dispatch_macro` at
`src/application/app/dispatch/macro_core.rs` (cross-platform,
abstracted over `MacroDispatchTarget`); the native shim that
wraps `InputHandlerContext` lives in
`src/application/app/dispatch/native.rs`, gated to native via the
file-level `#![cfg(not(target_arch = "wasm32"))]`.

**Track B took option (b) below: shape was the right one.** The
`dispatch_macro` step loop and privilege gate live in
`dispatch/macro_core.rs` (cross-platform); the native-only
Action-arm dispatcher in `dispatch/native.rs` stays cfg-gated.
WASM's `WasmMacroDispatchTarget` impl wraps `WasmInputState +
&mut Renderer` and reaches the SAME gate via the
`MacroDispatchTarget` trait. Recorded here for Track-A
contributors who might be tempted to re-implement the check
inline; the alternative was:

- **(a) Lift the cfg gate off the native dispatch module's
  declaration** and gate individual native-only arms instead. The
  privilege-enforcement code becomes cross-platform automatically.
  Not taken because the per-arm cfg-gate sprawl would be worse
  than the trait abstraction.

Re-implementing the privilege check inline is **forbidden** —
it's the threat-model defence and must be single-sourced. A
forked enforcement copy would silently drift when a future
contributor adds an Action to the denylist (`mod.rs:91-114`).

## Parametric verb actions

A subset of the parametric Action variants
(`Set<Concept>Field`-shaped — e.g. `SetBorderField`, `SetColorBg`,
`SetEdgeAnchor`, `SetSpacing`, `SetZoomMin`, `ClearZoom`,
`SetFontFamily`, `SetEdgeLabelText`, …) ride the same Track A
classification rules as their no-arg siblings: 20 variants are
`Compatible` because their bodies only touch
`MindMapDocument` setters, and 3 variants
(`OpenDocument`, `SaveDocumentAs`, `NewDocumentAt`) are
`NativeOnly` because they reach the filesystem via
`execute_console_line` → `loader::save_to_file` /
`MindMapDocument::load`.

The `Compatible` parametric arms are usable on WASM today
*through* `dispatch_action`; once Track A or Track C lands the
WASM-side dispatch funnel, no per-variant porting work is
required for them. The 3 fs variants stay deferred until WASM
gains a filesystem story (file-system-access API, IndexedDB
overlay, …) — tracked in TODO.md.

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
- Filesystem on WASM (`OpenDocument` / `SaveDocumentAs` /
  `NewDocumentAt` parametric Action variants stay `NativeOnly`
  pending a chosen storage strategy).
- **Touch / IME / Focused** input event arms — the catch-all in
  `WasmApp::handle_window_event` documents these by name; each
  needs its own `event_*.rs` sibling once wired. Touch is
  mobile-budget-binding (§4); IME is required for non-Latin
  text editing in the inline node-text editor.
- **Surfaced map-load error overlay.** The post-section loader
  rejects pre-migration maps with a concrete `maptool convert
  --sections` pointer; native users see the message on stderr
  but the WASM side currently routes it through `log::error!`
  only. A browser-only user with a stale map sees a stuck
  "loading…" frame instead of the actionable error. Adding a
  surfaced overlay (or a documented `?map=` failure UI) is the
  matching parity step. The data-shape rejection itself is
  cross-platform.
- **Maptool migration on WASM.** `maptool convert --sections`
  is native-only by construction (the binary doesn't ship to
  the browser). A browser-only authoring flow that loads a
  legacy map needs an in-app migration path or a documented
  desktop-side hop; today only the latter exists. See
  `format/sections.md` "Migration".

## Per-arm event-handler shape divergence

Wave 6C.1 split `WasmApp::handle_window_event` into per-event
sibling files at `src/application/app/run_wasm/event_*.rs`,
mirroring native's `src/application/app/event_*.rs` at file
granularity. The two layouts now diff cleanly side-by-side —
intentional, so a future Track-D reviewer can fold per-arm
bodies once the unified context is reachable.

But the function shape diverged:
- **Native** handlers are free functions taking `&mut
  InputHandlerContext<'_>` (e.g. `pub(super) fn
  handle_keyboard_input(logical_key, event_loop, ctx)` at
  `event_keyboard.rs:24`).
- **WASM** handlers are inherent methods on `WasmApp` (e.g.
  `impl WasmApp { pub(super) fn handle_keyboard_input(&mut
  self, logical_key) }` at `run_wasm/event_keyboard.rs`).

The WASM method shape is forced by the `Rc<RefCell<Option<…>>>`
cell projection — only inherent methods can do the
`borrow_mut().as_mut()` dance against `&mut self.input` /
`self.renderer` at call-site cost. A free function would
require the caller to do the projection and pass `&mut
WasmInputState` + `&mut Renderer` separately, which means
each arm body's call site grows by 5–8 lines of
`borrow_mut()` + `as_mut()` + early-return guards.

Track D's full convergence (collapse to "shared free functions
taking `&mut InputContextCore`") will need to either:
1. **Remove the cells**: convert `WasmApp` to own
   `Renderer` / `WasmInputState` directly. Possible since
   `WasmApp` is `'static` and the rAF render loop currently
   needs `Rc` clones — but Track D would replace the rAF loop
   shape too (e.g. drive rendering through
   `ApplicationHandler::about_to_wait`). Substantial refactor.
2. **Keep the cells, accept the method shape**: native's
   handlers grow `&mut self`-method shape too, with `self`
   wrapping a `&mut InputHandlerContext` stash. Less work but
   less elegant.

For now, the per-arm method shape is load-bearing on WASM and
diverges from native by design. New WASM event handlers added
before Track D should follow the method pattern.

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

1. [`src/application/keybinds/action/mod.rs`](./src/application/keybinds/action/mod.rs) —
   `Action::wasm_compatibility` is the API contract; the per-variant
   `#[action(wasm = ...)]` attributes drive
   `mandala_derive::ActionClassify`'s emitted classifier.
2. [`src/application/app/dispatch/native.rs`](./src/application/app/dispatch/native.rs) —
   the native dispatch funnel arms are the reference implementation.
   Cross-platform Compatible-arm bodies live in
   [`src/application/app/dispatch/cross_dispatch.rs`](./src/application/app/dispatch/cross_dispatch.rs).
3. [`src/application/app/input_context.rs`](./src/application/app/input_context.rs) —
   the 21-field context every native arm reads (passed by
   `&mut InputHandlerContext<'_>`).
4. [`src/application/app/run_wasm.rs`](./src/application/app/run_wasm.rs) —
   the WASM event loop with its inline match blocks. This file
   shrinks dramatically as Track A ports land.
5. [`format/macros.md`](./format/macros.md) — the privilege model
   that Track B must preserve.
