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
