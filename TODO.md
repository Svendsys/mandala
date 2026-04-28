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

- **Phase 9 — full WASM convergence.** `run_wasm.rs` still has its own
  partial copy of the keyboard `match action` block at lines 385-457
  and its own ClickHit-routing block at 523-635. WASM uses a
  different `InputState` struct from native's `InputHandlerContext`
  so unifying them needs a small refactor of the WASM input
  bookkeeping (`AppMode`, `LastClick` shapes) before `dispatch_action`
  can serve both targets. Until that lands, WASM users get the
  off-by-default empty-canvas behaviour but not the full mouse-
  gesture rebinding surface. **Macros are also native-only** for the
  same reason: `MacroRegistry` is built once on `InitState` in
  `run_native_init::build`, and WASM doesn't construct one. Plus
  there's no `~/.config` filesystem in a browser, so the loader
  would need a `?macros=<json>` / `localStorage` shape parallel to
  the keybind loader before shipping.
- **App-bundled and inline-on-map macro tiers.** Today only the user
  tier exists. Adding app-bundled macros (parallel to
  `assets/mutations/application.json`) is straightforward; adding an
  inline tier on `MindMap.macros` is the load-bearing one — once map
  files can carry macros, opening a hostile mindmap would let it run
  any `ConsoleLine` step (`save /tmp/evil`, `open ~/.bashrc`). The
  tier expansion **must** ship a `MacroSource` tag and gate
  `ConsoleLine` at the dispatcher to user-tier only. See
  CODE_CONVENTIONS.md §3 carve-out.
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
