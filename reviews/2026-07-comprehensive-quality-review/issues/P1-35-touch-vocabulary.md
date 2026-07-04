# P1-35: Touch vocabulary — no tap-select, no one-finger pan, no pinch-zoom; both shipped gestures default to NativeOnly Actions that drop silently on WASM (the primary touch target)

**Severity:** P1 (§4 "touch-first input is a peer" is unmet end-to-end) · **Area:** mandala/app touch + keybinds

## Problem

The touch recognizer (`src/application/app/touch_gesture.rs`) is a clean, well-tested plain-value state machine (§T9-compliant) — but it emits exactly two gestures: `LongPress` → default `EnterResizeMode`, `TwoFingerDrag` → default `FastResizeStart` (`keybinds/config.rs:297-298`). Both Actions are `NativeOnly`, so on WASM they dispatch, return `Unhandled`, and a one-shot warn fires (`run_wasm/event_touch.rs:81-92`). `run_wasm/mod.rs:148-152` calls WASM mobile "the *primary* surface this targets" — yet a phone user cannot tap-select, one-finger-pan, or pinch-zoom (winit's mouse synthesis explicitly doesn't cover these paths). CONCEPTS §5 still lists pinch/pan/long-press as "the next obvious user" — future tense for the flagship touch platform.

Also in this area (small, from the same reviews): the recognizer's `MOVE_THRESHOLD_PX = 4.0` claims to "mirror … same value, same intent" the mouse drag threshold, which is actually 5.0px (`DRAG_THRESHOLD_SQ_PX = 25.0` in `app/mod.rs:147`, different name and file than the comment claims) — unify or document the difference (single knob preferred); the per-move `.sqrt() > threshold` should be a squared compare like the mouse path (§B1 pointer-handler rule); `on_ended`'s comment narrates a reset that the code doesn't perform (the latch is what prevents refire).

## Fix plan

1. Extend `RecognizedGesture` with `Tap`, `PinchStep { center, scale_delta }`, and continuous two-finger-pan deltas — all as plain-value outputs of the existing machine (keeps §T9 testability).
2. Wire: `Tap` → the existing press/release selection path (synthesize through the shared click-hit core); `PinchStep`/pan deltas → `CameraZoom`/`CameraPan` decrees (per-frame continuous carve-out, mirroring mouse wheel/drag).
3. Give `LongPress`/`TwoFingerDrag` WASM-meaningful defaults or keep them native-bound deliberately — but then bind Tap/pinch/pan so the platform is usable; update the CONCEPTS entry.
4. Unify the drag-threshold constant (`POINTER_DRAG_THRESHOLD_PX`, squared derived); fix the recognizer comments.
5. Tests: recognizer emits Tap/Pinch/Pan for canonical touch sequences (pure state-machine tests, existing harness style); threshold parity test.

## Acceptance criteria

- On a touch device (or DevTools touch emulation) in the browser: tap selects, one-finger drag on empty canvas pans, pinch zooms.
- Recognizer tests cover the new gestures incl. two-finger jitter and finger-lift ordering.
- One drag-threshold constant.
- `./test.sh` green (wasm32 check included).

## Pointers

`src/application/app/touch_gesture.rs`; `src/application/app/run_wasm/event_touch.rs`; `src/application/app/run_native.rs:312-346` (native ingest twin — coordinate with P1-29's shared `drive_touch_event`); `src/application/keybinds/config.rs:297-298`; CODE_CONVENTIONS §4; CONCEPTS §5 (ThrottledInteraction "touch gestures are the next obvious user").
