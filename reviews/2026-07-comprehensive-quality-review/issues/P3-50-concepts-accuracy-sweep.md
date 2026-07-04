# P3-50: CONCEPTS.md accuracy sweep — the orientation document misdescribes the frame heartbeat, the dirty flag, rebuild tiers, dispatch mirrors, console verbs, and several counts

**Severity:** P3 (docs — but CONCEPTS is the mandated orientation doc; a fictitious mechanism actively misleads) · **Area:** CONCEPTS.md (+ two in-code stale refs)

## Verified drift list

1. **Frame heartbeat + dirty flag**: CONCEPTS §5 documents a drain order and a dirty-gated rebuild step ("Read at the top of drain_frame's rebuild step; reset at the bottom") that **does not exist** — `doc.dirty` is never read in drain code (grep: zero hits; it's a saved/unsaved tracker), the actual `drain_inputs` order differs, and `Renderer::process` runs from `RedrawRequested`, not the drain. Rewrite the "Event loop and drain_frame" + "Dirty flag" entries against `run_native.rs:557-709` / `drain_frame.rs`.
2. **"dispatch_custom_mutation_for_key mirrors the click-trigger path at click.rs:35-64 byte-for-byte"** — the trigger path moved to `click_triggers.rs`, the cited lines are now selection code, and the bodies have diverged (loop + section-aware `start_animation_at` vs single + `start_animation`). The stale `click.rs:35-64` pointer also lives in code (`dispatch/native.rs:972`, `cross_dispatch/mod.rs:82`) — fix those two comments too, and unify the two 3-branch timing-envelope bodies while touching them.
3. **Rebuild tiers**: CONCEPTS lists five; `scene_rebuild.rs` has nine role updaters + the selection-change chooser + section frames.
4. **Console verbs**: the list names `portal` and `quit` (neither exists; `portal` folded into `edge`) and omits the largest verbs (`border`, `canvas`, `section`, `node`, `mode`, `help`). Regenerate from `COMMANDS`.
5. **UndoAction count**: "12 variants" — code has 13 (`EditNodeAabb` missing from the list).
6. **Border default preset**: CONCEPTS says `"rounded"` is the default; code default is `"light"` with a documented rationale (`node.rs:503-509`).
7. **`Tree.position`/`pending_mutations` "used narrowly today"** — both are `#[allow(dead_code)]`-unused (align with the P2-43 outcome).
8. **`MutationApplicabilityGate`** — named in CONCEPTS, exists nowhere in code (the real carrier is `TriggerBinding.contexts`); CONCEPTS also cites `custom_mutation/document_action.rs` / `timing.rs` file paths that don't exist as named.
9. **Field/entry counts drifted**: `InputHandlerContext` "21 fields" (22); `input_context_core.rs` "the 11 fields" (12, plus a stale ext-list); `resolved.rs` "under 50 entries" (~110); `now_ms()` located in app/mod.rs (moved to `common`); `is_hidden_by_fold` "runs once per scene build" (runs per element per pass — align with P1-23's fix); LeftDrag "is dispatched through dispatch_action" (align with P1-31's fix); SelectionState::SectionRange grapheme semantics (align with P2-47's decision); Clipboard "WASM stubs warn-and-noop" (they `log::debug`); hit-priority documented as global but true only for drags (P2-48 item 10).
10. **Picker chips**: "theme-variable quick-pick chips … Tab cycles theme chips" — the chip row was retired (P2-41 removes the remnants).

## Fix plan

One sweep commit, ordered after the code-side issues it depends on land (or with "as of <commit>" wording where behavior is in flux). Where CONCEPTS documents intent the code should meet (e.g. Toggle behavior — P0-02), fix the code first and keep the doc.

## Acceptance criteria

- Every §5/§6 mechanism description matches a code path that exists, verified by the file:line references CONCEPTS itself cites.
- The two in-code stale refs corrected.

## Pointers

CONCEPTS.md; CODE_CONVENTIONS §8 ("a doc that lies is worse than no doc"); the input/document/console findings files for exact line numbers.
