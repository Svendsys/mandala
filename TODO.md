# Things that needs to be tested / troubleshot with a GPU

 - Nothing right now

# Deferred work from configurable-canvas-actions branch

The dispatch funnel and the user-visible mouse-gesture rebinding
(double-click, wheel zoom, middle-click pan, left-drag pan, custom-
mutation parity, 5 console-verb Actions) are wired and shipping.
Remaining phases from
`/root/.claude/plans/memoized-meandering-toucan.md` are scaffolded
but not yet wired into their modal handlers:

- **Phase 5 — TextEdit / LabelEdit cursor primitives.** The 13
  TextEdit + 6 LabelEdit cursor/delete Action variants exist in
  `Action` and resolve through `KeybindConfig`, but the modal
  handlers (`text_edit/editor.rs`, `label_edit.rs`) still own their
  hardcoded `Key::Named(NamedKey::ArrowLeft)` ladders. Wiring is
  mechanical — switch each ladder to
  `keybinds.action_for_context(InputContext::TextEdit/LabelEdit, ...)`
  + dispatch through `dispatch_action`. Each arm body is 2-3 lines
  wrapping the existing helpers in `text_edit/mod.rs` and
  `mod.rs:route_label_edit_key`.
- **Phase 8 — Macro scaffolding.** `Macro`, `MacroStep`,
  `MacroRegistry`, JSON loader, `dispatch_macro`, and a
  `macro_bindings: HashMap<String, String>` field on `KeybindConfig`.
  Modelled on the custom-mutation loader at
  `src/application/document/mutations_loader/`.
- **Phase 9 — WASM convergence.** `run_wasm.rs` still has its own
  partial copy of the keyboard `match action` block at lines 385-457
  and its own double-click ladder at 523-635. WASM uses a different
  `InputState` struct from native's `InputHandlerContext` so
  unifying them needs a small refactor of the WASM input
  bookkeeping (`AppMode`, `LastClick` shapes) before
  `dispatch_action` can serve both targets.
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
