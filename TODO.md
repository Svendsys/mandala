# Things that needs to be tested / troubleshot with a GPU

 - Nothing right now

# Deferred work from configurable-canvas-actions branch

Wired and shipping after the review-fix cycle:
- Dispatch funnel (Phases 1, 4)
- Mouse-gesture rebinding via extended KeyBind grammar (Phase 2) with
  modifier-fallback so Ctrl+Wheel etc. still work
- All ~46 Action variants (Phase 3) — most have functioning bodies
  after Phases 5/6
- 5 console-verb Actions (Phase 6)
- Custom-mutation keybind parity with the click path (Phase 7)
- TextEdit / LabelEdit cursor primitives (Phase 5)
- Macro scaffolding with user-layer JSON loader (Phase 8)
- Documentation (Phase 10) — CONCEPTS.md §5 "Action dispatch",
  CODE_CONVENTIONS.md §3 "Single dispatch funnel" with carve-outs

Phase 9 is partially done: WASM's empty-canvas double-click now also
honours the `CreateOrphanNodeAndEdit` opt-in gate (matches native).
The full WASM convergence below is still outstanding.

- **WASM convergence — full porting.** The foundation is in place:
  `Action::wasm_compatibility()` classifies every variant as
  `Compatible` or `NativeOnly`, the WASM keyboard handler filters on
  the classification, and `WASM_CONVERGENCE.md` documents the
  three porting tracks (port a NativeOnly Action, port the macro
  registry, unify the bundle/context type). Pick a track and walk
  it. The doc has the step-by-step recipe; the
  `Action::wasm_compatibility` rustdoc has the classification rules
  for new variants. WASM-side macro registry is the highest-value
  next step (Track B) because it unblocks every `Compatible`
  Action a user has bound to a macro id.
- **Shadow-stacked registry** *(reviewer follow-up)*. Today
  higher-tier macros DISPLACE lower-tier ones with the same id
  permanently within the session — so opening a Map-tier macro
  with the same id as a User-tier macro and then closing the
  document leaves the User entry gone. Documented in
  `format/macros.md` with namespacing recommendation, but a
  proper fix would store entries per-tier and resolve at lookup
  time. Substantial registry rewrite.
- **Parameterised console verbs as Actions.** `open <path>`,
  `save-as <path>`, `mutation apply <id>`, kv-shaped
  `border` / `edge` / `color` / `font` / `zoom` / `spacing` setters
  intentionally stay console-only — minting parameter-less Action
  stubs would be a half-feature per CODE_CONVENTIONS.md §5.
- **Action variants scaffolded with no dispatch arm.** These exist in
  the `Action` enum, have a config field + default + resolve-table
  entry, and parse from `keybinds.json` — but their dispatch arm
  hasn't been written yet. `dispatch_action`'s catch-all
  `_ => Unhandled` swallows them with a debug log. All ship with
  `vec![]` defaults so user-facing behaviour is "binding does
  nothing." Wiring is per-arm work:
  - **Navigation/camera:** `ZoomReset`, `ZoomFit`, `PanCameraNorth`,
    `PanCameraSouth`, `PanCameraEast`, `PanCameraWest`,
    `CenterOnSelection`, `JumpToRoot`. Each needs a small arm body
    using `Renderer::set_camera_center` / `RenderDecree::CameraPan` /
    `Renderer::fit_camera_to_tree`.
  - **Selection:** `SelectAll`, `DeselectAll`, `InvertSelection`,
    `SelectParent`, `SelectChild`, `SelectNextSibling`,
    `SelectPrevSibling`. Each needs a small arm body using existing
    `MindMap` parent/children traversal.
  - **Editor commit-on-click:** `CommitOrCloseEditor` reified for the
    click-outside path in `event_mouse_click.rs:425-563`; that path
    still calls `close_text_edit` / `close_label_edit` /
    `close_portal_text_edit` directly. Wiring the Action would
    consolidate three identical "click-outside commits" branches.
  - **Document lifecycle:** `NewDocument` (no path) is mostly wireable
    by mimicking the `new` console verb's `replace_document` field
    drain, but needs the `ConsoleEffects` drain refactor referenced
    in Phase 6 of the plan. Parameterised `open <path>` / `save-as
    <path>` stay console-only by design.
- **InputHandlerContext rebuild duplication.** Four sites manually
  reconstruct `InputHandlerContext` from destructured locals before
  calling `dispatch_action`: `event_keyboard.rs` (twice — action +
  custom-mutation fall-through), `event_mouse_click.rs` (twice —
  Middle-click + DoubleClick). The rebuild exists because both
  handler functions destructure `ctx` at entry for borrow-split
  reasons. Cleaner shape: drop the destructure, take
  `ctx: &mut InputHandlerContext<'_>` and pass through directly.
  Mechanical refactor.
