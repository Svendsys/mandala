# P1-31: Dispatch-funnel gaps — Picker commit/cancel/nudge Actions never reach the funnel; LeftDrag looks up but doesn't dispatch; EditSelection arm duplicated with a dead `_clean` flag

**Severity:** P1 (violates the codebase's own §3 rule; macros can't drive the picker) · **Area:** mandala/app/dispatch

## Problem A — Picker actions executed outside dispatch_action

`dispatch/native.rs:637-641` states the house rule: "Commit/cancel are user-named effects … NOT the §3 carve-out for literal Key character insertion — so they belong in the funnel," and TextEdit/LabelEdit commit/cancel are funnel-dispatched accordingly. But `PickerCancel`, `PickerCommit`, and the six `PickerNudge*` Actions are executed entirely inside `handle_color_picker_key` (`color_picker_flow/key.rs:44-106`); `dispatch_action` has **no arms** for them (falls to `_ => Unhandled`). Consequence: `MacroStep::Action { PickerCommit }` is a silent no-op while `TextEditCommit` from a macro works — the third modal is inconsistent with the other two, and the macro/plugin trajectory can't drive the picker.

**Fix:** move the eight Picker* bodies into `dispatch_action`; `handle_color_picker_key` pre-filters like the label/text editors, keeping only Copy/Paste/Cut modal-specifics + literal-char fallthrough local.

## Problem B — LeftDrag: gesture looked up, body inlined, never dispatched

`event_cursor_moved.rs:546-573`: `action_for_gesture(LeftDrag) == Some(Action::PanCanvas)` is checked, then `DragState::Panning` is set **inline** — `dispatch_action` is never called, and the `PanCanvas` arm body (`native.rs:563-570`) is duplicated. The site itself admits "future Actions bound to `LeftDrag` won't fire here without explicit handling." CONCEPTS §5 claims LeftDrag "is dispatched through dispatch_action" — false. (The per-frame pan delta staying inline is the sanctioned carve-out; the discrete entry is what the funnel covers. Compare MiddleClick, which dispatches correctly.)

**Fix:** `if let Some(a) = action { dispatch_action(a, ctx, None); }` then emit the first pan delta if the state became `Panning`. Fix the CONCEPTS sentence if behavior intentionally differs.

## Problem C — EditSelection fall-through and LabelEditOnSelection are the same body; `_clean` is a dead contract

`native.rs:400-434` vs `601-634`: both arms match selection → `PortalLabel|PortalText → open_portal_text_edit`, `EdgeLabel → open_label_edit`, identically. `native.rs:407`: `let _clean = matches!(action, Action::EditSelectionClean);` — computed and **discarded**, so `EditSelectionClean` on an edge-label/portal selection opens the editor with existing text, diverging from its documented "empty buffer" contract (a half-feature marker).

**Fix:** extract `open_editor_for_edge_selection(clean, ctx)` used by both arms; thread `clean` into the single-line editors (seed empty buffer) — or delete the flag and document the divergence on the Action. Coordinate with P1-30 (same editor files).

## Acceptance criteria

- `MacroStep::Action` can drive every picker action (test: macro fires PickerCommit while picker open → color committed).
- Rebinding LeftDrag to a different Action fires that Action.
- `EditSelectionClean` honors its empty-buffer contract on every selection kind (or the contract is removed).
- `./test.sh` green.

## Pointers

CODE_CONVENTIONS §3 (single dispatch funnel + exact carve-out list); CONCEPTS §5 (Action dispatch); files cited inline.
