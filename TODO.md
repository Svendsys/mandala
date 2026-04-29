# Things that needs to be tested / troubleshot with a GPU

 - Nothing right now

# Deferred work from configurable-canvas-actions branch

Shipped on this branch:
- Dispatch funnel: `dispatch_action`, `dispatch_macro`,
  `dispatch_custom_mutation_for_key`. Native dispatch goes through
  the funnel; mouse and keyboard handlers feed into it via gesture-
  name lookup.
- Mouse-gesture rebinding via extended `KeyBind` grammar with
  modifier-fallback for `Ctrl+Wheel` etc.
- ~70 `Action` variants with bodies for navigation (zoom, pan,
  fit, jump-to-root, center-on-selection), selection
  (`SelectAll/DeselectAll/Invert`, `SelectParent/Child/Sibling*`),
  TextEdit / LabelEdit cursor primitives, and several no-arg
  console verbs (`OpenColorPicker`, `ToggleFps`, `ToggleFpsDebug`,
  `LabelEditOnSelection`, `NewDocument`).
- Custom-mutation keybind parity with the click-trigger path
  (animation-aware, `apply_document_actions` envelope).
- Macro scaffolding: four-tier registry (App / User / Map / Inline),
  shadow-stacked storage so higher tiers reveal lower tiers when
  cleared, fail-closed privilege gate, JSON loader.
- Privilege gate is structurally enforced: `Action::is_destructive`
  is an exhaustive match the compiler enforces against
  `#[non_exhaustive]` `Action`. New variants cannot land without an
  explicit destructive / non-destructive classification.
- Cross-platform `apply_text_edit_action` and the cursor / word
  primitives moved to `text_edit/mod.rs` so the WASM editor can
  reach them through `text_edit::` directly.
- WASM keyboard handler honours empty-canvas double-click opt-in
  gate; `EditSelection` / `EditSelectionClean` Single-selection
  branch fires on WASM through a pre-filter exception.

## Outstanding

- ~~**WASM convergence — full funnel.**~~ **Track C shipped**
  — both targets now dispatch every Compatible Action through
  the same cross-platform `dispatch_action_core::dispatch_compatible`
  function. WASM-only `dispatch_compatible_action_wasm` shim
  deleted (-320 LoC). `InputContextCore` (cross-platform 11
  fields) + `NativeContextExt` (native-only 10 fields) split via
  `InputHandlerContext::split_borrow` on native; WASM builds a
  core directly via `WasmInputState::input_context_core`. Mixed-
  branch arms (`CancelMode`, `EditSelection*`) run their
  cross-platform slice in the unified dispatcher and return
  `Unhandled` so native fall-through can run the residual
  AppMode / EdgeLabel / Portal native-only branches.
- **Native dead-arm cleanup**: the Compatible arms in
  `dispatch.rs::dispatch_action`'s match (Undo, ZoomIn, etc.)
  are now unreachable — the cross-platform dispatcher handles
  them first via the delegation shim. Left in place this branch
  for behaviour-preservation safety; a focused follow-up can
  delete them and shrink the file.
- ~~**WASM Compatible Actions need arms.**~~ **Track A largely
  shipped.** A new `cross_dispatch` module (partial Track C) holds
  the Action arm bodies that touch only state shared between
  native and WASM; both dispatchers call the same per-action
  helpers. Wired across two batches: A.1 (camera + selection +
  FPS — 16 arms) and A.2 (parametric — 20 arms). Copy/Cut/Paste
  remain WASM-side no-ops via the cfg-stubbed `clipboard` module
  — wiring those will become meaningful when async web-clipboard
  integration lands.
- ~~**Parameterised console verbs as Actions.**~~ **Shipped.**
  23 parametric Action variants now span anchor / body / border /
  cap / color / edge / font / label / spacing / zoom / filesystem.
  `ParametricBinding { combo, args }` is the binding shape;
  per-variant resolve closures pick payloads apart; mutation
  cores extracted from each verb file are reused by both the
  verb path (with scrollback) and the Action arm (no scrollback).
  Filesystem variants (`OpenDocument`, `SaveDocumentAs`,
  `NewDocumentAt`) are NativeOnly + denylisted from non-User
  macro tiers per the privilege gate. `mutation apply <id>`
  stays console-only — already covered by
  `custom_mutation_bindings`.
- **Reparent / Connect target-click handlers bypass the funnel.**
  `event_mouse_click.rs` calls `handle_reparent_target_click` /
  `handle_connect_target_click` directly. Both push undo entries
  but aren't `Action` variants — they should be.
- **Modal commit/cancel inline in modal handlers.** `text_edit`,
  `label_edit`, `portal_text_edit` each have their own commit /
  cancel branches in their modal handler bodies; only `TextEdit`
  Cancel routes through the funnel. Folding the rest is a §3
  cleanup.
- **Console-verb Action bodies inline in `console_input/dispatch.rs`.**
  Every `Action::Console*` variant is matched and run inline at
  the console handler; none reach `dispatch_action`. Either route
  through the funnel or document the carve-out clearly.
- **`word_left` / `word_right` belong in baumhard.**
  `text_edit/mod.rs` houses these primitives today, but per
  CODE_CONVENTIONS §B3 text primitives extend
  `lib/baumhard/src/util/grapheme_chad.rs`. Cross-crate move +
  bench per §B3.
