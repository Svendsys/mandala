# P1-29: Native/WASM input duplication — double-click/create-orphan/click-outside-commit/touch/init bodies copied; WASM wheel bypasses the keybind funnel; WASM drops the custom-mutation tier

**Severity:** P1 (§4 platform parity + §3 funnel violations; several fixes are unblocked and small) · **Area:** mandala/app

## Context

`work_plans/WASM_CONVERGENCE.md` tracks input-pipeline unification and already acknowledges the double-click ladder as "the largest remaining Track-A duplication". This issue captures the full verified inventory — several items are NOT in that plan's deferred list and are independently shippable.

## Problems

1. **Duplicated behavior bodies** (each at both sites): double-click-on-node → section-aware select + rebuild + open editor (`dispatch/native.rs:454-491` vs `run_wasm/event_mouse_click.rs:138-171`, the latter marked "See the parallel native path"); double-click portal → pan-to-partner (`native.rs:493-521` vs wasm:172-195); create-orphan-and-edit body exists **3×** (`native.rs:1141-1162`, `cross_dispatch/lifecycle.rs:56-72` — the designated core! — and wasm:247-266 inline); text-edit click-outside-commit (`event_mouse_click.rs:403-447` vs wasm:330-379); already-editing guard (native 3-editor form vs wasm reduced copy); touch ingest→tick→dispatch ~20-line near-copy (`run_native.rs:312-346` vs `run_wasm/event_touch.rs:38-97`); load-time init incl. handle-tree warm + macro-registry build with identical logging (`run_native_init.rs:58-230` vs `run_wasm/mod.rs:660-824`, five "mirrors native" comments).
2. **WASM double-click never consults the keybind table** — native resolves `MouseGesture::DoubleClick` → Action; WASM hardcodes the behavior, so rebinding/unbinding `double_click_activate` is silently ignored in the browser.
3. **WASM wheel zoom bypasses the funnel entirely** (`run_wasm/event_mouse_wheel.rs:19-52`): hardcoded `factor = 1.1`, direct `CameraZoom` decree + synchronous `rebuild_scene_only`, no `action_for_gesture(WheelUp/WheelDown)` — while `ZoomIn`/`ZoomOut` are `Compatible` and `dispatch_compatible` is already wired on WASM. The `scroll_y` decomposition is also copied verbatim from native. Not blocked on convergence work.
4. **WASM keyboard chain drops the custom-mutation tier** (`run_wasm/event_keyboard.rs:117-142`): native resolves Action → Macro → CustomMutation; WASM stops at Macro, though `custom_mutation_for` and `apply_keybind_custom_mutation` are already cross-platform and invoked on WASM via the macro path. A `custom_mutation_bindings` entry works on desktop, silently dead in the browser. Only the ~25-line lookup shim is native-gated.
5. **Stale promised tests**: `app/tests.rs:99-111` promises guard-predicate tests that don't exist ("We verify the guard predicate here" — followed by nothing); the predicate was never extracted, so the promise is structurally unfulfillable.

## Fix plan

1. Extract plain-value cores in `cross_dispatch` (they touch only `InputContextCore` fields): `apply_double_click_activate(hit, core, text_edit_state)`; make WASM call the existing `apply_create_orphan_node_and_edit` and delete the other two copies; `commit_text_edit_on_outside_release(core, canvas)`; `already_editing_same_target(...) -> bool` (then write the promised tests); `drive_touch_event(recognizer, keybinds, phase, id, pos, now) -> Option<Action>`; `warm_scene_at_load(...)` + `build_macro_registry(doc)`.
2. Route both targets' double-click and wheel through `action_for_gesture` + `dispatch_compatible`; extract shared `wheel_lines(delta) -> f64`.
3. Lift `dispatch_custom_mutation_for_key` onto `InputContextCore` and call it from the WASM fall-through.
4. Update `work_plans/WASM_CONVERGENCE.md` to reflect what's closed (or fold the remainder into it and reference from there).

## Acceptance criteria

- Rebinding `double_click_activate` and wheel gestures works identically on both targets.
- `custom_mutation_bindings` fire on WASM.
- Grep shows one body per behavior (native + wasm both call the core).
- `./test.sh` green (wasm32 check included); new guard tests exist.

## Pointers

Files cited inline; `work_plans/WASM_CONVERGENCE.md`; CODE_CONVENTIONS §3 (funnel), §4 (+§T9: platform-shared logic as plain-value functions).
